// SPDX-License-Identifier: GPL-3.0-only

use super::reports::{
    ProviderHealthReport, ProviderInspectFallbackRow, ProviderInspectReport,
    ProviderProfileCatalogRow, ProviderProfilesReport,
};
use super::shared::{
    apply_configured_model_cost, format_marker_list, format_retry_without_parameters,
    provider_debug_live_smoke_state, provider_debug_profile_source,
};
use crate::builder::host_agent_context;
use crate::provider_probe::model_provider_live_probe;
use ikaros_core::{IkarosPaths, ModelConfig, Result, redact_secrets};
use ikaros_providers::model::{ProviderHealthLedger, ProviderRegistry};
use std::path::Path;
pub fn provider_inspect_report(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ProviderInspectReport> {
    let host = host_agent_context(paths, workspace, agent_override)?;
    let config = &host.config;
    let agent = &host.agent_instance;
    let model = agent.model_config(&config.model.default);
    let model_provider =
        agent.effective_model_provider_config(&config.model.default, &config.providers.model);
    let mut descriptor = ProviderRegistry.descriptor_with_profile(
        &model.provider,
        &model_provider.base_url,
        &model.model,
        &model.compat_profile,
    )?;
    apply_configured_model_cost(&mut descriptor, &model.cost);
    let fallback_rows = provider_inspect_fallback_rows(model)?;

    Ok(ProviderInspectReport {
        provider: descriptor.provider.clone(),
        model: redact_secrets(&descriptor.model),
        configured_profile: redact_secrets(&model.compat_profile),
        profile: descriptor.profile.clone(),
        profile_source: provider_debug_profile_source(
            &model.provider,
            Some(&model.compat_profile),
            Some(&descriptor),
        ),
        temperature_policy: descriptor.profile_policy.temperature.clone(),
        reasoning_policy: descriptor.profile_policy.reasoning.clone(),
        message_policy: descriptor.profile_policy.message.clone(),
        tool_schema_policy: descriptor.profile_policy.tool_schema.clone(),
        request_body_policy: descriptor.profile_policy.request_body.clone(),
        retry_without_parameters: format_retry_without_parameters(
            &descriptor.profile_policy.retry_without_parameters,
        ),
        fallback_rows,
        context_window: descriptor.context.context_window,
        default_output_tokens: descriptor.context.default_output_tokens,
        tokenizer: format!("{:?}", descriptor.context.tokenizer),
        streaming: descriptor.capabilities.streaming,
        tool_calls: descriptor.capabilities.tool_calls,
        reasoning: descriptor.capabilities.reasoning,
        json_mode: descriptor.capabilities.json_mode,
        network: descriptor.capabilities.network,
        image_input: descriptor.capabilities.image_input,
        audio_input: descriptor.capabilities.audio_input,
        file_input: descriptor.capabilities.file_input,
        health: format!("{:?}", descriptor.health.status),
        cost_input_per_million: descriptor.cost.input_per_million,
        cost_output_per_million: descriptor.cost.output_per_million,
        cost_cache_read_per_million: descriptor.cost.cache_read_per_million,
        cost_cache_write_per_million: descriptor.cost.cache_write_per_million,
        cost_currency: descriptor.cost.currency.clone(),
    })
}

pub fn provider_profiles_report() -> ProviderProfilesReport {
    let registry = ProviderRegistry;
    let rows = registry
        .openai_compatible_profile_catalog()
        .into_iter()
        .map(|profile| ProviderProfileCatalogRow {
            provider: redact_secrets(&profile.provider),
            profile: redact_secrets(&profile.profile),
            auto_base_url_markers: format_marker_list(&profile.auto_base_url_markers),
            auto_model_markers: format_marker_list(&profile.auto_model_markers),
            auto_model_tail_prefixes: format_marker_list(&profile.auto_model_tail_prefixes),
            temperature_policy: redact_secrets(&profile.profile_policy.temperature),
            reasoning_policy: redact_secrets(&profile.profile_policy.reasoning),
            message_policy: redact_secrets(&profile.profile_policy.message),
            tool_schema_policy: redact_secrets(&profile.profile_policy.tool_schema),
            request_body_policy: redact_secrets(&profile.profile_policy.request_body),
            retry_without_parameters: format_retry_without_parameters(
                &profile.profile_policy.retry_without_parameters,
            ),
            context_window: profile.context.context_window,
            default_output_tokens: profile.context.default_output_tokens,
            tokenizer: format!("{:?}", profile.context.tokenizer),
            streaming: profile.capabilities.streaming,
            tool_calls: profile.capabilities.tool_calls,
            reasoning: profile.capabilities.reasoning,
            json_mode: profile.capabilities.json_mode,
            network: profile.capabilities.network,
            image_input: profile.capabilities.image_input,
            audio_input: profile.capabilities.audio_input,
            file_input: profile.capabilities.file_input,
        })
        .collect();
    ProviderProfilesReport {
        provider: "openai-compatible".into(),
        rows,
    }
}

pub async fn provider_health_report(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    live: bool,
) -> Result<ProviderHealthReport> {
    let host = host_agent_context(paths, workspace, agent_override)?;
    let config = &host.config;
    let agent = &host.agent_instance;
    let model = agent.model_config(&config.model.default);
    let model_provider =
        agent.effective_model_provider_config(&config.model.default, &config.providers.model);
    if live {
        return Ok(
            match model_provider_live_probe(
                paths,
                workspace,
                config,
                model,
                &model_provider,
                "Ikaros provider health probe. Reply with a short ok.",
            )
            .await
            {
                Ok(response) => ProviderHealthReport::LiveOk {
                    provider: response.provider,
                    model: redact_secrets(&response.model),
                    usage_total: response.usage_total,
                },
                Err(error) => ProviderHealthReport::LiveFailed {
                    error: redact_secrets(&error.to_string()),
                },
            },
        );
    }

    let ledger = ProviderHealthLedger::new(&paths.audit_dir);
    let latest = ledger.latest(&model.provider, &model.model)?;
    Ok(ProviderHealthReport::Local {
        provider: model.provider.to_string(),
        model: redact_secrets(&model.model),
        health: latest
            .as_ref()
            .map(|record| format!("{:?}", record.status))
            .unwrap_or_else(|| "Unknown".into()),
        consecutive_failures: latest
            .as_ref()
            .map(|record| record.consecutive_failures)
            .unwrap_or_default(),
        last_error_kind: latest
            .as_ref()
            .and_then(|record| record.last_error_kind.map(|kind| format!("{kind:?}"))),
        last_error_summary: latest.as_ref().and_then(|record| {
            (!record.last_error_summary.is_empty())
                .then(|| redact_secrets(&record.last_error_summary))
        }),
        cooldown_until: latest
            .as_ref()
            .and_then(|record| record.cooldown_until.clone()),
        health_log: ledger.path().to_path_buf(),
    })
}
fn provider_inspect_fallback_rows(model: &ModelConfig) -> Result<Vec<ProviderInspectFallbackRow>> {
    let registry = ProviderRegistry;
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
            let base_url_configured = !fallback_provider.base_url.trim().is_empty();
            let api_key_configured = !fallback_provider.api_key.trim().is_empty();
            Ok(ProviderInspectFallbackRow {
                index,
                provider: redact_secrets(&descriptor.provider),
                model: redact_secrets(&descriptor.model),
                configured_profile: redact_secrets(&fallback_model.compat_profile),
                profile: redact_secrets(&descriptor.profile),
                live_smoke: provider_debug_live_smoke_state(
                    &fallback_model.provider,
                    &fallback_model.model,
                    base_url_configured,
                    api_key_configured,
                ),
                streaming: descriptor.capabilities.streaming,
                tool_calls: descriptor.capabilities.tool_calls,
                reasoning: descriptor.capabilities.reasoning,
                network: descriptor.capabilities.network,
                image_input: descriptor.capabilities.image_input,
                audio_input: descriptor.capabilities.audio_input,
                file_input: descriptor.capabilities.file_input,
                context_window: descriptor.context.context_window,
                default_output_tokens: descriptor.context.default_output_tokens,
            })
        })
        .collect()
}
