// SPDX-License-Identifier: GPL-3.0-only

use super::reports::{
    WorkbenchModelBudgetStatus, WorkbenchModelCostStatus, WorkbenchProviderFallbackStatus,
    WorkbenchProviderHealthStatus, WorkbenchProviderStatusReport,
};
use super::shared::{
    apply_configured_model_cost, format_optional_cost, provider_debug_live_smoke_state,
    provider_debug_profile_source, provider_matrix_capability, provider_matrix_context_window,
    provider_matrix_default_output_tokens, provider_matrix_policy, provider_matrix_profile,
    provider_matrix_retry_without_parameters, provider_matrix_tokenizer,
    suggested_daily_token_budget,
};
use super::usage::{ProviderMatrixUsageSummary, provider_matrix_usage_summary};
use ikaros_core::{IkarosPaths, ModelConfig, RemoteProviderConfig, Result, redact_secrets};
use ikaros_providers::model::{
    ModelProviderDescriptor, ModelUsageLedger, ProviderHealthLedger, ProviderHealthRecord,
    ProviderRegistry,
};
pub fn model_budget_status_report(
    paths: &IkarosPaths,
    model: &ModelConfig,
) -> Result<WorkbenchModelBudgetStatus> {
    let usage = ModelUsageLedger::new(&paths.audit_dir);
    workbench_model_budget_status(&usage, model)
}

pub fn workbench_provider_status_report(
    paths: &IkarosPaths,
    model: &ModelConfig,
    model_provider: &RemoteProviderConfig,
) -> Result<WorkbenchProviderStatusReport> {
    let registry = ProviderRegistry;
    let health_ledger = ProviderHealthLedger::new(&paths.audit_dir);
    let usage = ModelUsageLedger::new(&paths.audit_dir);
    let mut descriptor = registry
        .descriptor_with_profile(
            &model.provider,
            &model_provider.base_url,
            &model.model,
            &model.compat_profile,
        )
        .ok();
    if let Some(descriptor) = &mut descriptor {
        apply_configured_model_cost(descriptor, &model.cost);
    }
    let health_record = health_ledger.latest(&model.provider, &model.model)?;
    let usage_today =
        provider_matrix_usage_summary(&usage, &model.provider, &model.model, descriptor.as_ref())?;
    let budget = workbench_model_budget_status(&usage, model)?;
    let cost = workbench_model_cost_status(descriptor.as_ref(), &usage_today);
    let fallbacks = workbench_provider_fallback_statuses(&registry, model);

    Ok(WorkbenchProviderStatusReport {
        provider: redact_secrets(&model.provider.to_string()),
        model: redact_secrets(&model.model),
        configured_profile: redact_secrets(&model.compat_profile),
        provider_profile: provider_matrix_profile(&descriptor),
        profile_source: provider_debug_profile_source(
            &model.provider,
            Some(&model.compat_profile),
            descriptor.as_ref(),
        ),
        context_window: provider_matrix_context_window(&descriptor),
        default_output_tokens: provider_matrix_default_output_tokens(&descriptor),
        tokenizer: provider_matrix_tokenizer(&descriptor),
        runtime: redact_secrets(&model.runtime),
        transport: redact_secrets(&model.transport.to_string()),
        temperature_policy: provider_matrix_policy(&descriptor, |policy| &policy.temperature),
        reasoning_policy: provider_matrix_policy(&descriptor, |policy| &policy.reasoning),
        message_policy: provider_matrix_policy(&descriptor, |policy| &policy.message),
        tool_schema_policy: provider_matrix_policy(&descriptor, |policy| &policy.tool_schema),
        request_body_policy: provider_matrix_policy(&descriptor, |policy| &policy.request_body),
        prompt_cache_policy: provider_matrix_policy(&descriptor, |policy| &policy.prompt_cache),
        retry_without_parameters: provider_matrix_retry_without_parameters(&descriptor),
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
        health: workbench_provider_health_status(health_record.as_ref()),
        budget,
        cost,
        fallback_count: model.fallbacks.len(),
        fallback_chain: workbench_provider_fallback_chain(model),
        fallbacks,
    })
}
fn workbench_model_budget_status(
    usage: &ModelUsageLedger,
    model: &ModelConfig,
) -> Result<WorkbenchModelBudgetStatus> {
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let used_today = usage.total_for_day(&today)?;
    let (remaining_today, budget_status) = match model.daily_token_budget {
        Some(budget) => {
            let remaining = budget.saturating_sub(used_today);
            let status = if used_today >= budget {
                "exhausted"
            } else if used_today.saturating_mul(10) >= budget.saturating_mul(9) {
                "near_limit"
            } else {
                "ok"
            };
            (Some(remaining), status)
        }
        None => (None, "unbounded"),
    };
    Ok(WorkbenchModelBudgetStatus {
        daily_token_budget: model.daily_token_budget,
        used_today,
        remaining_today,
        budget_status,
        suggested_daily_token_budget: suggested_daily_token_budget(
            model.daily_token_budget,
            used_today,
        ),
    })
}

fn workbench_model_cost_status(
    descriptor: Option<&ModelProviderDescriptor>,
    usage_today: &ProviderMatrixUsageSummary,
) -> WorkbenchModelCostStatus {
    let Some(descriptor) = descriptor else {
        return WorkbenchModelCostStatus {
            currency: "unknown".into(),
            input_per_million: "unknown".into(),
            output_per_million: "unknown".into(),
            cache_read_per_million: "unknown".into(),
            cache_write_per_million: "unknown".into(),
            estimated_cost_today: "unknown".into(),
            cache_read_tokens_today: usage_today.cache_read_tokens,
            cache_write_tokens_today: usage_today.cache_write_tokens,
            cache_accounting: "unavailable",
        };
    };
    WorkbenchModelCostStatus {
        currency: redact_secrets(&descriptor.cost.currency),
        input_per_million: format_optional_cost(descriptor.cost.input_per_million),
        output_per_million: format_optional_cost(descriptor.cost.output_per_million),
        cache_read_per_million: format_optional_cost(descriptor.cost.cache_read_per_million),
        cache_write_per_million: format_optional_cost(descriptor.cost.cache_write_per_million),
        estimated_cost_today: usage_today.estimated_cost_today.clone(),
        cache_read_tokens_today: usage_today.cache_read_tokens,
        cache_write_tokens_today: usage_today.cache_write_tokens,
        cache_accounting: match usage_today.cache_accounting {
            "unavailable" => "tracked",
            accounting => accounting,
        },
    }
}

fn workbench_provider_health_status(
    health: Option<&ProviderHealthRecord>,
) -> WorkbenchProviderHealthStatus {
    WorkbenchProviderHealthStatus {
        status: health
            .map(|record| format!("{:?}", record.status))
            .unwrap_or_else(|| "Unknown".into()),
        consecutive_failures: health
            .map(|record| record.consecutive_failures)
            .unwrap_or_default(),
        last_error_kind: health
            .and_then(|record| record.last_error_kind.map(|kind| format!("{kind:?}")))
            .unwrap_or_else(|| "none".into()),
        last_error_summary: health
            .map(|record| redact_secrets(&record.last_error_summary))
            .unwrap_or_default(),
        cooldown_until: health
            .and_then(|record| record.cooldown_until.clone())
            .unwrap_or_else(|| "none".into()),
    }
}

fn workbench_provider_fallback_statuses(
    registry: &ProviderRegistry,
    model: &ModelConfig,
) -> Vec<WorkbenchProviderFallbackStatus> {
    model
        .fallbacks
        .iter()
        .enumerate()
        .map(|(index, fallback)| {
            let fallback_model = fallback.model_config();
            let fallback_provider = fallback.provider_config();
            let descriptor = registry
                .descriptor_with_profile(
                    &fallback_model.provider,
                    &fallback_provider.base_url,
                    &fallback_model.model,
                    &fallback_model.compat_profile,
                )
                .ok();
            WorkbenchProviderFallbackStatus {
                index,
                provider: redact_secrets(&fallback_model.provider.to_string()),
                model: redact_secrets(&fallback_model.model),
                configured_profile: redact_secrets(&fallback_model.compat_profile),
                provider_profile: provider_matrix_profile(&descriptor),
                live_smoke: provider_debug_live_smoke_state(
                    &fallback_model.provider,
                    &fallback_model.model,
                    !fallback_provider.base_url.trim().is_empty(),
                    !fallback_provider.api_key.trim().is_empty(),
                ),
                streaming: provider_matrix_capability(&descriptor, |descriptor| {
                    descriptor.capabilities.streaming
                }),
                tool_calls: provider_matrix_capability(&descriptor, |descriptor| {
                    descriptor.capabilities.tool_calls
                }),
                reasoning: provider_matrix_capability(&descriptor, |descriptor| {
                    descriptor.capabilities.reasoning
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
                context_window: provider_matrix_context_window(&descriptor),
                default_output_tokens: provider_matrix_default_output_tokens(&descriptor),
            }
        })
        .collect()
}

fn workbench_provider_fallback_chain(model: &ModelConfig) -> String {
    if model.fallbacks.is_empty() {
        return "none".into();
    }
    model
        .fallbacks
        .iter()
        .enumerate()
        .map(|(index, fallback)| {
            format!(
                "{}:{}/{} profile={}",
                index,
                redact_secrets(&fallback.provider.to_string()),
                redact_secrets(&fallback.model),
                redact_secrets(&fallback.compat_profile)
            )
        })
        .collect::<Vec<_>>()
        .join(" -> ")
}
