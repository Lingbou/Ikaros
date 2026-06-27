// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{ModelConfig, ModelCostConfig, redact_secrets};
use ikaros_providers::model::{ModelProviderDescriptor, ModelProviderProfilePolicy};

pub(super) fn suggested_daily_token_budget(current: Option<u32>, used: u32) -> u32 {
    let usage_headroom = used.saturating_add(100_000);
    let configured_headroom = current
        .map(|budget| budget.saturating_mul(2))
        .unwrap_or(100_000);
    usage_headroom.max(configured_headroom).max(100_000)
}

pub(super) fn format_optional_cost(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn provider_matrix_fallback_role(kind: &str) -> &'static str {
    if kind == "model" {
        "primary"
    } else {
        "not-applicable"
    }
}

pub(super) fn provider_matrix_profile(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| redact_secrets(&descriptor.profile))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn provider_matrix_policy(
    descriptor: &Option<ModelProviderDescriptor>,
    read: impl FnOnce(&ModelProviderProfilePolicy) -> &str,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| redact_secrets(read(&descriptor.profile_policy)))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn provider_matrix_retry_without_parameters(
    descriptor: &Option<ModelProviderDescriptor>,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| {
            format_retry_without_parameters(&descriptor.profile_policy.retry_without_parameters)
        })
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn provider_matrix_context_window(
    descriptor: &Option<ModelProviderDescriptor>,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| descriptor.context.context_window.to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn provider_matrix_default_output_tokens(
    descriptor: &Option<ModelProviderDescriptor>,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| descriptor.context.default_output_tokens.to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn provider_matrix_tokenizer(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| format!("{:?}", descriptor.context.tokenizer))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn provider_matrix_capability(
    descriptor: &Option<ModelProviderDescriptor>,
    read: impl FnOnce(&ModelProviderDescriptor) -> bool,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| read(descriptor).to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn provider_matrix_cost(
    descriptor: &Option<ModelProviderDescriptor>,
    read: impl FnOnce(&ModelProviderDescriptor) -> Option<f64>,
) -> String {
    descriptor
        .as_ref()
        .and_then(read)
        .map(|cost| cost.to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn provider_debug_fallback_model_names(model: &ModelConfig) -> Vec<String> {
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
pub(super) fn apply_configured_model_cost(
    descriptor: &mut ModelProviderDescriptor,
    cost: &ModelCostConfig,
) {
    if !model_cost_is_configured(cost) {
        return;
    }
    descriptor.cost.currency = redact_secrets(&cost.currency);
    if let Some(input) = cost.input_per_million {
        descriptor.cost.input_per_million = Some(input);
    }
    if let Some(output) = cost.output_per_million {
        descriptor.cost.output_per_million = Some(output);
    }
    if let Some(cache_read) = cost.cache_read_per_million {
        descriptor.cost.cache_read_per_million = Some(cache_read);
    }
    if let Some(cache_write) = cost.cache_write_per_million {
        descriptor.cost.cache_write_per_million = Some(cache_write);
    }
}

pub(super) fn model_cost_is_configured(cost: &ModelCostConfig) -> bool {
    cost.currency.trim() != "USD"
        || cost.input_per_million.is_some()
        || cost.output_per_million.is_some()
        || cost.cache_read_per_million.is_some()
        || cost.cache_write_per_million.is_some()
}

pub(super) fn format_retry_without_parameters(parameters: &[String]) -> String {
    if parameters.is_empty() {
        "none".into()
    } else {
        redact_secrets(&parameters.join(","))
    }
}

pub(super) fn format_marker_list(markers: &[String]) -> String {
    if markers.is_empty() {
        "none".into()
    } else {
        redact_secrets(&markers.join(","))
    }
}

pub(super) fn provider_debug_configured_profile(
    provider: &str,
    configured_profile: Option<&str>,
) -> String {
    if provider.trim().eq_ignore_ascii_case("openai-compatible") {
        return redact_secrets(configured_profile.unwrap_or("auto"));
    }
    "native".into()
}

pub(super) fn provider_debug_profile_source(
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

pub(super) fn provider_debug_live_smoke_state(
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

pub(super) fn provider_debug_hint(live_smoke: &str) -> &'static str {
    match live_smoke {
        "ready" => "ready",
        "offline" | "local-ready" => "offline-provider",
        "missing-model" => "configure-model",
        "missing-base-url" => "configure-base-url",
        "missing-api-key" => "configure-api-key",
        _ => "inspect-provider",
    }
}
