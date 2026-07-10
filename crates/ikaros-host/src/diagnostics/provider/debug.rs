// SPDX-License-Identifier: GPL-3.0-only

use super::reports::ProviderDebugMatrixReport;
use super::shared::{
    provider_debug_configured_profile, provider_debug_fallback_model_names, provider_debug_hint,
    provider_debug_live_smoke_state, provider_debug_profile_source,
};
use super::usage::provider_debug_usage_summary;
use crate::builder::host_agent_context;
use ikaros_core::{AgentInstance, IkarosConfig, IkarosPaths, ModelConfig, Result, redact_secrets};
use ikaros_providers::model::{
    ModelProviderDescriptor, ModelUsageLedger, ProviderHealthLedger, ProviderRegistry,
};
use serde_json::{Value, json};
use std::path::Path;
pub fn provider_debug_report(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Value> {
    let host = host_agent_context(paths, workspace, agent_override)?;
    let config = &host.config;
    let agent = &host.agent_instance;
    let model = agent.model_config(&config.model.default);
    let model_provider =
        agent.effective_model_provider_config(&config.model.default, &config.providers.model);
    let registry = ProviderRegistry;
    let health = ProviderHealthLedger::new(&paths.audit_dir);
    let descriptor = registry.descriptor_with_profile(
        &model.provider,
        &model_provider.base_url,
        &model.model,
        &model.compat_profile,
    )?;
    let matrix = provider_debug_matrix_report(config, agent, &paths.audit_dir)?;
    Ok(json!({
        "format": "ikaros-provider-debug-v1",
        "workspace": &agent.workspace,
        "agent_id": &agent.agent_id,
        "profile": &agent.profile_name,
        "health_log": matrix.health_log.display().to_string(),
        "model": provider_debug_model_summary(
            &descriptor,
            model,
            model_provider.base_url.trim(),
            model_provider.api_key.trim(),
            &health,
        ),
        "fallback_chain": provider_debug_fallback_chain(&registry, model)?,
        "matrix": matrix.rows,
    }))
}

pub fn provider_debug_matrix_report(
    config: &IkarosConfig,
    agent: &AgentInstance,
    audit_dir: &Path,
) -> Result<ProviderDebugMatrixReport> {
    let registry = ProviderRegistry;
    let health = ProviderHealthLedger::new(audit_dir);
    let usage = ModelUsageLedger::new(audit_dir);
    let rows = provider_debug_matrix(config, agent, &registry, &health, &usage)?;
    Ok(ProviderDebugMatrixReport {
        health_log: health.path().to_path_buf(),
        rows,
    })
}

fn provider_debug_model_summary(
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

fn provider_debug_policy(descriptor: &ModelProviderDescriptor) -> Value {
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

fn provider_debug_health(health: &ProviderHealthLedger, provider: &str, model: &str) -> Value {
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

fn provider_debug_fallback_chain(
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
fn provider_debug_matrix(
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
    ]
    .into_iter()
    .collect()
}

struct ProviderDebugMatrixInput<'a> {
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

fn provider_debug_matrix_row(input: ProviderDebugMatrixInput<'_>) -> Result<Value> {
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
