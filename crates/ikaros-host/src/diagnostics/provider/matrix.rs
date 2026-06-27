// SPDX-License-Identifier: GPL-3.0-only

use super::reports::{ProviderMatrixReport, ProviderMatrixRow};
use super::shared::{
    apply_configured_model_cost, provider_debug_configured_profile,
    provider_debug_fallback_model_names, provider_debug_hint, provider_debug_live_smoke_state,
    provider_debug_profile_source, provider_matrix_capability, provider_matrix_context_window,
    provider_matrix_cost, provider_matrix_default_output_tokens, provider_matrix_fallback_role,
    provider_matrix_policy, provider_matrix_profile, provider_matrix_retry_without_parameters,
    provider_matrix_tokenizer,
};
use super::usage::provider_matrix_usage_summary;
use crate::builder::host_agent_context;
use crate::provider_probe::{
    asr_provider_live_probe, embedding_provider_live_probe, model_provider_live_probe,
    tts_provider_live_probe,
};
use ikaros_core::{
    IkarosConfig, IkarosPaths, ModelConfig, ModelCostConfig, Result, redact_secrets,
};
use ikaros_providers::model::{ModelUsageLedger, ProviderHealthLedger, ProviderRegistry};
use std::path::Path;
pub async fn provider_matrix_report(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    live: bool,
) -> Result<ProviderMatrixReport> {
    let host = host_agent_context(paths, workspace, agent_override)?;
    let config = &host.config;
    let agent = &host.agent_instance;
    let model_config = agent.model_config(&config.model.default);
    let model_provider =
        agent.effective_model_provider_config(&config.model.default, &config.providers.model);
    let model_live_probe = if live {
        provider_matrix_model_live_probe(paths, workspace, config, model_config, &model_provider)
            .await
    } else {
        ProviderMatrixLiveProbe::not_run()
    };
    let embedding_live_probe = if live {
        provider_matrix_embedding_live_probe(paths, workspace, config).await
    } else {
        ProviderMatrixLiveProbe::not_run()
    };
    let tts_live_probe = if live {
        provider_matrix_tts_live_probe(workspace, config).await
    } else {
        ProviderMatrixLiveProbe::not_run()
    };
    let asr_live_probe = if live {
        provider_matrix_asr_live_probe(workspace, config).await
    } else {
        ProviderMatrixLiveProbe::not_run()
    };
    let registry = ProviderRegistry;
    let health = ProviderHealthLedger::new(&paths.audit_dir);
    let usage = ModelUsageLedger::new(&paths.audit_dir);
    let rows = vec![
        provider_matrix_row(ProviderMatrixRowInput {
            registry: &registry,
            health: &health,
            usage: &usage,
            kind: "model",
            provider: &model_config.provider.to_string(),
            model: &model_config.model,
            base_url: &model_provider.base_url,
            api_key: &model_provider.api_key,
            compat_profile: Some(&model_config.compat_profile),
            live_probe: &model_live_probe,
            fallback_models: provider_debug_fallback_model_names(model_config),
            configured_cost: Some(&model_config.cost),
        }),
        provider_matrix_row(ProviderMatrixRowInput {
            registry: &registry,
            health: &health,
            usage: &usage,
            kind: "embedding",
            provider: &config.rag.embedding_provider,
            model: &config.rag.embedding_model,
            base_url: &config.providers.embedding.base_url,
            api_key: &config.providers.embedding.api_key,
            compat_profile: None,
            live_probe: &embedding_live_probe,
            fallback_models: Vec::new(),
            configured_cost: None,
        }),
        provider_matrix_row(ProviderMatrixRowInput {
            registry: &registry,
            health: &health,
            usage: &usage,
            kind: "tts",
            provider: &config.voice.tts.provider,
            model: &config.voice.tts.model,
            base_url: &config.providers.tts.base_url,
            api_key: &config.providers.tts.api_key,
            compat_profile: None,
            live_probe: &tts_live_probe,
            fallback_models: Vec::new(),
            configured_cost: None,
        }),
        provider_matrix_row(ProviderMatrixRowInput {
            registry: &registry,
            health: &health,
            usage: &usage,
            kind: "asr",
            provider: &config.voice.asr.provider,
            model: &config.voice.asr.model,
            base_url: &config.providers.asr.base_url,
            api_key: &config.providers.asr.api_key,
            compat_profile: None,
            live_probe: &asr_live_probe,
            fallback_models: Vec::new(),
            configured_cost: None,
        }),
    ]
    .into_iter()
    .collect::<Result<Vec<_>>>()?;
    Ok(ProviderMatrixReport { live, rows })
}
#[derive(Debug, Clone)]
struct ProviderMatrixLiveProbe {
    status: String,
    detail: String,
}

impl ProviderMatrixLiveProbe {
    fn ok(detail: impl Into<String>) -> Self {
        Self {
            status: "ok".into(),
            detail: redact_secrets(&detail.into()),
        }
    }

    fn failed(detail: impl Into<String>) -> Self {
        Self {
            status: "failed".into(),
            detail: redact_secrets(&detail.into()),
        }
    }

    fn not_run() -> Self {
        Self {
            status: "not-run".into(),
            detail: "not-run".into(),
        }
    }
}

async fn provider_matrix_model_live_probe(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
    model_config: &ModelConfig,
    model_provider: &ikaros_core::RemoteProviderConfig,
) -> ProviderMatrixLiveProbe {
    match model_provider_live_probe(
        paths,
        workspace,
        config,
        model_config,
        model_provider,
        "Ikaros provider matrix live probe. Reply with ok.",
    )
    .await
    {
        Ok(response) => {
            ProviderMatrixLiveProbe::ok(format!("usage_total={}", response.usage_total))
        }
        Err(error) => ProviderMatrixLiveProbe::failed(error.to_string()),
    }
}

async fn provider_matrix_embedding_live_probe(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
) -> ProviderMatrixLiveProbe {
    match embedding_provider_live_probe(paths, workspace, config).await {
        Ok(detail) => ProviderMatrixLiveProbe::ok(detail),
        Err(error) => ProviderMatrixLiveProbe::failed(error.to_string()),
    }
}

async fn provider_matrix_tts_live_probe(
    workspace: &Path,
    config: &IkarosConfig,
) -> ProviderMatrixLiveProbe {
    match tts_provider_live_probe(workspace, config).await {
        Ok(detail) => ProviderMatrixLiveProbe::ok(detail),
        Err(error) => ProviderMatrixLiveProbe::failed(error.to_string()),
    }
}

async fn provider_matrix_asr_live_probe(
    workspace: &Path,
    config: &IkarosConfig,
) -> ProviderMatrixLiveProbe {
    match asr_provider_live_probe(workspace, config).await {
        Ok(detail) => ProviderMatrixLiveProbe::ok(detail),
        Err(error) => ProviderMatrixLiveProbe::failed(error.to_string()),
    }
}

struct ProviderMatrixRowInput<'a> {
    registry: &'a ProviderRegistry,
    health: &'a ProviderHealthLedger,
    usage: &'a ModelUsageLedger,
    kind: &'a str,
    provider: &'a str,
    model: &'a str,
    base_url: &'a str,
    api_key: &'a str,
    compat_profile: Option<&'a str>,
    live_probe: &'a ProviderMatrixLiveProbe,
    fallback_models: Vec<String>,
    configured_cost: Option<&'a ModelCostConfig>,
}

fn provider_matrix_row(input: ProviderMatrixRowInput<'_>) -> Result<ProviderMatrixRow> {
    let mut descriptor = match input.compat_profile {
        Some(compat_profile) => input.registry.descriptor_with_profile(
            input.provider,
            input.base_url,
            input.model,
            compat_profile,
        ),
        None => input
            .registry
            .descriptor(input.provider, input.base_url, input.model),
    }
    .ok();
    if let (Some(descriptor), Some(cost)) = (&mut descriptor, input.configured_cost) {
        apply_configured_model_cost(descriptor, cost);
    }
    let health_record = input
        .health
        .latest(input.provider, input.model)
        .ok()
        .flatten();
    let base_url_configured = !input.base_url.trim().is_empty();
    let api_key_configured = !input.api_key.trim().is_empty();
    let live_smoke = provider_debug_live_smoke_state(
        input.provider,
        input.model,
        base_url_configured,
        api_key_configured,
    );
    let usage_today = provider_matrix_usage_summary(
        input.usage,
        input.provider,
        input.model,
        descriptor.as_ref(),
    )?;
    Ok(ProviderMatrixRow {
        kind: redact_secrets(input.kind),
        provider: redact_secrets(input.provider),
        model: redact_secrets(input.model),
        base_url_configured,
        api_key_configured,
        live_smoke,
        live_probe: redact_secrets(&input.live_probe.status),
        probe_detail: redact_secrets(&input.live_probe.detail),
        health_status: health_record
            .as_ref()
            .map(|record| format!("{:?}", record.status))
            .unwrap_or_else(|| "Unknown".into()),
        consecutive_failures: health_record
            .as_ref()
            .map(|record| record.consecutive_failures)
            .unwrap_or_default(),
        cooldown_until: health_record
            .as_ref()
            .and_then(|record| record.cooldown_until.clone()),
        configured_profile: provider_debug_configured_profile(input.provider, input.compat_profile),
        provider_profile: provider_matrix_profile(&descriptor),
        profile_source: provider_debug_profile_source(
            input.provider,
            input.compat_profile,
            descriptor.as_ref(),
        ),
        temperature_policy: provider_matrix_policy(&descriptor, |policy| &policy.temperature),
        reasoning_policy: provider_matrix_policy(&descriptor, |policy| &policy.reasoning),
        message_policy: provider_matrix_policy(&descriptor, |policy| &policy.message),
        tool_schema_policy: provider_matrix_policy(&descriptor, |policy| &policy.tool_schema),
        request_body_policy: provider_matrix_policy(&descriptor, |policy| &policy.request_body),
        retry_without_parameters: provider_matrix_retry_without_parameters(&descriptor),
        context_window: provider_matrix_context_window(&descriptor),
        default_output_tokens: provider_matrix_default_output_tokens(&descriptor),
        tokenizer: provider_matrix_tokenizer(&descriptor),
        streaming: provider_matrix_capability(&descriptor, |descriptor| {
            descriptor.capabilities.streaming
        }),
        tool_calls: provider_matrix_capability(&descriptor, |descriptor| {
            descriptor.capabilities.tool_calls
        }),
        reasoning: provider_matrix_capability(&descriptor, |descriptor| {
            descriptor.capabilities.reasoning
        }),
        json_mode: provider_matrix_capability(&descriptor, |descriptor| {
            descriptor.capabilities.json_mode
        }),
        network: provider_matrix_capability(&descriptor, |descriptor| {
            descriptor.capabilities.network
        }),
        image_input: provider_matrix_capability(&descriptor, |descriptor| {
            descriptor.capabilities.image_input
        }),
        audio_input: provider_matrix_capability(&descriptor, |descriptor| {
            descriptor.capabilities.audio_input
        }),
        file_input: provider_matrix_capability(&descriptor, |descriptor| {
            descriptor.capabilities.file_input
        }),
        cost_input_per_million: provider_matrix_cost(&descriptor, |descriptor| {
            descriptor.cost.input_per_million
        }),
        cost_output_per_million: provider_matrix_cost(&descriptor, |descriptor| {
            descriptor.cost.output_per_million
        }),
        cost_cache_read_per_million: provider_matrix_cost(&descriptor, |descriptor| {
            descriptor.cost.cache_read_per_million
        }),
        cost_cache_write_per_million: provider_matrix_cost(&descriptor, |descriptor| {
            descriptor.cost.cache_write_per_million
        }),
        cost_currency: descriptor
            .as_ref()
            .map(|descriptor| descriptor.cost.currency.clone())
            .unwrap_or_else(|| "unknown".into()),
        usage_requests_today: usage_today.requests,
        usage_prompt_tokens_today: usage_today.prompt_tokens,
        usage_completion_tokens_today: usage_today.completion_tokens,
        usage_total_tokens_today: usage_today.total_tokens,
        cache_read_tokens_today: usage_today.cache_read_tokens,
        cache_write_tokens_today: usage_today.cache_write_tokens,
        estimated_cost_today: usage_today.estimated_cost_today,
        cache_accounting: usage_today.cache_accounting,
        fallback_role: provider_matrix_fallback_role(input.kind),
        fallback_count: input.fallback_models.len(),
        fallback_models: input
            .fallback_models
            .into_iter()
            .map(|model| redact_secrets(&model))
            .collect(),
        debug_hint: provider_debug_hint(live_smoke),
    })
}
