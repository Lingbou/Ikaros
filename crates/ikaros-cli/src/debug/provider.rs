// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) fn debug_provider(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let model = agent.model_config(&config.model.default);
    let model_provider =
        agent.effective_model_provider_config(&config.model.default, &config.providers.model);
    let registry = ProviderRegistry;
    let health = ProviderHealthLedger::new(&paths.audit_dir);
    let usage = ModelUsageLedger::new(&paths.audit_dir);
    let descriptor = registry.descriptor_with_profile(
        &model.provider,
        &model_provider.base_url,
        &model.model,
        &model.compat_profile,
    )?;
    let matrix = vec![
        provider_debug_matrix_row(ProviderDebugMatrixInput {
            registry: &registry,
            health: &health,
            kind: "model",
            provider: &model.provider,
            model: &model.model,
            base_url: &model_provider.base_url,
            api_key: &model_provider.api_key,
            compat_profile: Some(&model.compat_profile),
            fallback_models: provider_debug_fallback_model_names(model),
            usage: &usage,
        }),
        provider_debug_matrix_row(ProviderDebugMatrixInput {
            registry: &registry,
            health: &health,
            kind: "embedding",
            provider: &config.rag.embedding_provider,
            model: &config.rag.embedding_model,
            base_url: &config.providers.embedding.base_url,
            api_key: &config.providers.embedding.api_key,
            compat_profile: None,
            fallback_models: Vec::new(),
            usage: &usage,
        }),
        provider_debug_matrix_row(ProviderDebugMatrixInput {
            registry: &registry,
            health: &health,
            kind: "tts",
            provider: &config.voice.tts.provider,
            model: &config.voice.tts.model,
            base_url: &config.providers.tts.base_url,
            api_key: &config.providers.tts.api_key,
            compat_profile: None,
            fallback_models: Vec::new(),
            usage: &usage,
        }),
        provider_debug_matrix_row(ProviderDebugMatrixInput {
            registry: &registry,
            health: &health,
            kind: "asr",
            provider: &config.voice.asr.provider,
            model: &config.voice.asr.model,
            base_url: &config.providers.asr.base_url,
            api_key: &config.providers.asr.api_key,
            compat_profile: None,
            fallback_models: Vec::new(),
            usage: &usage,
        }),
    ]
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    let output = json!({
        "format": "ikaros-provider-debug-v1",
        "workspace": agent.workspace,
        "agent_id": agent.agent_id,
        "profile": agent.profile_name,
        "health_log": health.path().display().to_string(),
        "model": provider_debug_model_summary(
            &descriptor,
            model,
            model_provider.base_url.trim(),
            model_provider.api_key.trim(),
            &health,
        ),
        "fallback_chain": provider_debug_fallback_chain(&registry, model)?,
        "matrix": matrix,
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(in crate::debug) fn provider_debug_model_summary(
    descriptor: &ModelProviderDescriptor,
    model: &ModelConfig,
    base_url: &str,
    api_key: &str,
    health: &ProviderHealthLedger,
) -> Value {
    json!({
        "provider": redact_secrets(&descriptor.provider),
        "model": redact_secrets(&descriptor.model),
        "configured_profile": redact_secrets(&model.compat_profile),
        "provider_profile": redact_secrets(&descriptor.profile),
        "profile_source": provider_debug_profile_source(
            &model.provider,
            Some(&model.compat_profile),
            Some(descriptor),
        ),
        "base_url_configured": !base_url.is_empty(),
        "api_key_configured": !api_key.is_empty(),
        "live_smoke": provider_debug_live_smoke_state(
            &model.provider,
            &model.model,
            !base_url.is_empty(),
            !api_key.is_empty(),
        ),
        "policy": provider_debug_policy(descriptor),
        "context": descriptor.context,
        "capabilities": descriptor.capabilities,
        "cost": descriptor.cost,
        "health": provider_debug_health(health, &model.provider, &model.model),
    })
}

pub(in crate::debug) fn provider_debug_policy(descriptor: &ModelProviderDescriptor) -> Value {
    json!({
        "temperature": redact_secrets(&descriptor.profile_policy.temperature),
        "reasoning": redact_secrets(&descriptor.profile_policy.reasoning),
        "message": redact_secrets(&descriptor.profile_policy.message),
        "tool_schema": redact_secrets(&descriptor.profile_policy.tool_schema),
        "request_body": redact_secrets(&descriptor.profile_policy.request_body),
        "prompt_cache": redact_secrets(&descriptor.profile_policy.prompt_cache),
        "retry_without_parameters": descriptor.profile_policy.retry_without_parameters,
    })
}

pub(in crate::debug) fn provider_debug_health(
    health: &ProviderHealthLedger,
    provider: &str,
    model: &str,
) -> Value {
    let record = health.latest(provider, model).ok().flatten();
    json!({
        "status": record
            .as_ref()
            .map(|record| format!("{:?}", record.status))
            .unwrap_or_else(|| "Unknown".into()),
        "consecutive_failures": record
            .as_ref()
            .map(|record| record.consecutive_failures)
            .unwrap_or(0),
        "last_error_kind": record.as_ref().and_then(|record| record.last_error_kind),
        "last_error_summary": record
            .as_ref()
            .map(|record| redact_secrets(&record.last_error_summary))
            .unwrap_or_default(),
        "cooldown_until": record.as_ref().and_then(|record| record.cooldown_until.clone()),
    })
}

pub(in crate::debug) fn provider_debug_fallback_chain(
    registry: &ProviderRegistry,
    model: &ModelConfig,
) -> Result<Vec<Value>> {
    model
        .fallbacks
        .iter()
        .enumerate()
        .map(|(index, fallback)| {
            let fallback_model = fallback.model_config();
            let fallback_provider = fallback.provider_config();
            let descriptor = registry.descriptor_with_profile(
                &fallback_model.provider,
                &fallback_provider.base_url,
                &fallback_model.model,
                &fallback_model.compat_profile,
            )?;
            Ok(json!({
                "index": index,
                "provider": redact_secrets(&descriptor.provider),
                "model": redact_secrets(&descriptor.model),
                "configured_profile": redact_secrets(&fallback_model.compat_profile),
                "provider_profile": redact_secrets(&descriptor.profile),
                "profile_source": provider_debug_profile_source(
                    &fallback_model.provider,
                    Some(&fallback_model.compat_profile),
                    Some(&descriptor),
                ),
                "live_smoke": provider_debug_live_smoke_state(
                    &fallback_model.provider,
                    &fallback_model.model,
                    !fallback_provider.base_url.trim().is_empty(),
                    !fallback_provider.api_key.trim().is_empty(),
                ),
                "context": descriptor.context,
                "capabilities": descriptor.capabilities,
            }))
        })
        .collect()
}

pub(in crate::debug) fn provider_debug_fallback_model_names(model: &ModelConfig) -> Vec<String> {
    model
        .fallbacks
        .iter()
        .map(|fallback| {
            if fallback.model.trim().is_empty() {
                fallback.provider.to_string()
            } else {
                fallback.model.clone()
            }
        })
        .collect()
}

pub(in crate::debug) fn provider_debug_matrix(
    config: &IkarosConfig,
    agent: &AgentInstance,
    registry: &ProviderRegistry,
    health: &ProviderHealthLedger,
    usage: &ModelUsageLedger,
) -> Result<Vec<Value>> {
    let model = agent.model_config(&config.model.default);
    let model_provider =
        agent.effective_model_provider_config(&config.model.default, &config.providers.model);
    vec![
        provider_debug_matrix_row(ProviderDebugMatrixInput {
            registry,
            health,
            kind: "model",
            provider: &model.provider,
            model: &model.model,
            base_url: &model_provider.base_url,
            api_key: &model_provider.api_key,
            compat_profile: Some(&model.compat_profile),
            fallback_models: provider_debug_fallback_model_names(model),
            usage,
        }),
        provider_debug_matrix_row(ProviderDebugMatrixInput {
            registry,
            health,
            kind: "embedding",
            provider: &config.rag.embedding_provider,
            model: &config.rag.embedding_model,
            base_url: &config.providers.embedding.base_url,
            api_key: &config.providers.embedding.api_key,
            compat_profile: None,
            fallback_models: Vec::new(),
            usage,
        }),
        provider_debug_matrix_row(ProviderDebugMatrixInput {
            registry,
            health,
            kind: "tts",
            provider: &config.voice.tts.provider,
            model: &config.voice.tts.model,
            base_url: &config.providers.tts.base_url,
            api_key: &config.providers.tts.api_key,
            compat_profile: None,
            fallback_models: Vec::new(),
            usage,
        }),
        provider_debug_matrix_row(ProviderDebugMatrixInput {
            registry,
            health,
            kind: "asr",
            provider: &config.voice.asr.provider,
            model: &config.voice.asr.model,
            base_url: &config.providers.asr.base_url,
            api_key: &config.providers.asr.api_key,
            compat_profile: None,
            fallback_models: Vec::new(),
            usage,
        }),
    ]
    .into_iter()
    .collect()
}
pub(in crate::debug) struct ProviderDebugMatrixInput<'a> {
    registry: &'a ProviderRegistry,
    health: &'a ProviderHealthLedger,
    kind: &'a str,
    provider: &'a str,
    model: &'a str,
    base_url: &'a str,
    api_key: &'a str,
    compat_profile: Option<&'a str>,
    fallback_models: Vec<String>,
    usage: &'a ModelUsageLedger,
}

pub(in crate::debug) fn provider_debug_matrix_row(
    input: ProviderDebugMatrixInput<'_>,
) -> Result<Value> {
    let descriptor = match input.compat_profile {
        Some(profile) => input
            .registry
            .descriptor_with_profile(input.provider, input.base_url, input.model, profile)
            .ok(),
        None => input
            .registry
            .descriptor(input.provider, input.base_url, input.model)
            .ok(),
    };
    let base_url_configured = !input.base_url.trim().is_empty();
    let api_key_configured = !input.api_key.trim().is_empty();
    let usage_today = provider_debug_usage_summary(
        input.usage,
        input.provider,
        input.model,
        descriptor.as_ref(),
    )?;
    Ok(json!({
        "kind": redact_secrets(input.kind),
        "provider": redact_secrets(input.provider),
        "model": redact_secrets(input.model),
        "base_url_configured": base_url_configured,
        "api_key_configured": api_key_configured,
        "live_smoke": provider_debug_live_smoke_state(
            input.provider,
            input.model,
            base_url_configured,
            api_key_configured,
        ),
        "configured_profile": provider_debug_configured_profile(input.provider, input.compat_profile),
        "provider_profile": descriptor
            .as_ref()
            .map(|descriptor| redact_secrets(&descriptor.profile))
            .unwrap_or_else(|| "unknown".into()),
        "profile_source": provider_debug_profile_source(input.provider, input.compat_profile, descriptor.as_ref()),
        "health": provider_debug_health(input.health, input.provider, input.model),
        "context": descriptor.as_ref().map(|descriptor| json!(descriptor.context)),
        "capabilities": descriptor.as_ref().map(|descriptor| json!(descriptor.capabilities)),
        "cost": descriptor.as_ref().map(|descriptor| json!(descriptor.cost)),
        "usage_today": usage_today,
        "fallback_role": if input.kind == "model" { "primary" } else { "not-applicable" },
        "fallback_count": input.fallback_models.len(),
        "fallback_models": input
            .fallback_models
            .iter()
            .map(|model| redact_secrets(model))
            .collect::<Vec<_>>(),
        "debug_hint": provider_debug_hint(provider_debug_live_smoke_state(
            input.provider,
            input.model,
            base_url_configured,
            api_key_configured,
        )),
    }))
}

pub(in crate::debug) fn provider_debug_usage_summary(
    usage: &ModelUsageLedger,
    provider: &str,
    model: &str,
    descriptor: Option<&ModelProviderDescriptor>,
) -> Result<Value> {
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let records = usage.read_all()?;
    let today_records = records
        .iter()
        .filter(|record| {
            record.at.starts_with(&today) && record.provider == provider && record.model == model
        })
        .collect::<Vec<_>>();
    let prompt_tokens = today_records
        .iter()
        .map(|record| record.prompt_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let completion_tokens = today_records
        .iter()
        .map(|record| record.completion_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let total_tokens = today_records
        .iter()
        .map(|record| record.total_tokens as u64)
        .sum::<u64>();
    let cache_read_tokens = today_records
        .iter()
        .map(|record| record.cache_read_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let cache_write_tokens = today_records
        .iter()
        .map(|record| record.cache_write_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let estimated_records = today_records
        .iter()
        .filter(|record| record.estimated)
        .count();
    let cost = descriptor.map(|descriptor| &descriptor.cost);
    let estimated_cost_today =
        cost.and_then(|cost| provider_debug_estimated_cost_today(&today_records, cost));
    Ok(json!({
        "day": today,
        "requests": today_records.len(),
        "estimated_records": estimated_records,
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": total_tokens,
        "cache_read_tokens": cache_read_tokens,
        "cache_write_tokens": cache_write_tokens,
        "currency": cost.map(|cost| redact_secrets(&cost.currency)),
        "estimated_cost_today": estimated_cost_today,
        "cache_accounting": cost
            .map(provider_debug_cache_accounting)
            .unwrap_or("unavailable"),
    }))
}

pub(in crate::debug) fn provider_debug_estimated_cost_today(
    records: &[&ModelUsageRecord],
    cost: &ikaros_models::ModelProviderCost,
) -> Option<String> {
    let (Some(input), Some(output)) = (cost.input_per_million, cost.output_per_million) else {
        return None;
    };
    let prompt_tokens = records
        .iter()
        .map(|record| record.prompt_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let completion_tokens = records
        .iter()
        .map(|record| record.completion_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let cache_read_tokens = records
        .iter()
        .map(|record| record.cache_read_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let cache_write_tokens = records
        .iter()
        .map(|record| record.cache_write_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let cache_read_price = cost.cache_read_per_million.unwrap_or(input);
    let cache_write_price = cost.cache_write_per_million.unwrap_or(input);
    let regular_input_tokens =
        (prompt_tokens - cache_read_tokens - cache_write_tokens).clamp(0.0, f64::MAX);
    Some(format!(
        "{:.6}",
        ((regular_input_tokens * input)
            + (completion_tokens * output)
            + (cache_read_tokens * cache_read_price)
            + (cache_write_tokens * cache_write_price))
            / 1_000_000.0
    ))
}

pub(in crate::debug) fn provider_debug_cache_accounting(
    cost: &ikaros_models::ModelProviderCost,
) -> &'static str {
    if cost.cache_read_per_million.is_some() || cost.cache_write_per_million.is_some() {
        "priced"
    } else if cost.input_per_million.is_some() || cost.output_per_million.is_some() {
        "tracked"
    } else {
        "unavailable"
    }
}

pub(in crate::debug) fn provider_debug_configured_profile(
    provider: &str,
    configured_profile: Option<&str>,
) -> String {
    if provider.trim().eq_ignore_ascii_case("openai-compatible") {
        return redact_secrets(configured_profile.unwrap_or("auto"));
    }
    "native".into()
}

pub(in crate::debug) fn provider_debug_profile_source(
    provider: &str,
    configured_profile: Option<&str>,
    descriptor: Option<&ModelProviderDescriptor>,
) -> &'static str {
    if provider.trim().eq_ignore_ascii_case("openai-compatible") {
        let configured = configured_profile
            .unwrap_or("auto")
            .trim()
            .to_ascii_lowercase();
        if configured.is_empty() || configured == "auto" {
            return match descriptor.map(|descriptor| descriptor.profile.as_str()) {
                Some("generic") => "auto-fallback",
                Some(_) => "auto-detected",
                None => "auto",
            };
        }
        return "explicit";
    }
    "native"
}

pub(in crate::debug) fn provider_debug_live_smoke_state(
    provider: &str,
    model: &str,
    base_url_configured: bool,
    api_key_configured: bool,
) -> &'static str {
    match provider {
        "mock" | "hash" => "offline",
        "ollama" => {
            if model.trim().is_empty() {
                "missing-model"
            } else {
                "local-ready"
            }
        }
        _ if model.trim().is_empty() => "missing-model",
        _ if !base_url_configured => "missing-base-url",
        _ if !api_key_configured => "missing-api-key",
        _ => "ready",
    }
}

pub(in crate::debug) fn provider_debug_hint(live_smoke: &str) -> &'static str {
    match live_smoke {
        "ready" => "ready",
        "offline" | "local-ready" => "offline-provider",
        "missing-model" => "configure-model",
        "missing-base-url" => "configure-base-url",
        "missing-api-key" => "configure-api-key",
        _ => "inspect-provider",
    }
}
