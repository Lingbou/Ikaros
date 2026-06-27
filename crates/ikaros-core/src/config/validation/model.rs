// SPDX-License-Identifier: GPL-3.0-only

use super::{
    super::{ModelConfig, ModelCostConfig, ModelFallbackConfig, RemoteProviderConfig},
    ConfigValidationReport, normalize, validate_float_range, validate_optional_positive,
    validate_optional_url, validate_required, validate_required_url, validate_timeout,
};

pub(super) fn validate_model_config(
    model_path: &str,
    provider_path: &str,
    config: &ModelConfig,
    provider_settings: &RemoteProviderConfig,
    report: &mut ConfigValidationReport,
) {
    if let Some(preset_id) = config.preset.as_deref() {
        if let Err(err) = crate::preset::resolve_preset(preset_id) {
            report.error(format!("{model_path}.preset"), err.to_string());
        }
    }

    let provider = normalize(&config.provider);
    if config.runtime.trim() != "harness-agent-loop" {
        report.error(
            format!("{model_path}.runtime"),
            "only `harness-agent-loop` is supported",
        );
    }
    validate_model_transport(model_path, &provider, &config.transport, report);
    validate_timeout(
        format!("{model_path}.timeout_ms"),
        config.timeout_ms,
        report,
    );
    validate_optional_positive(
        format!("{model_path}.rate_limit_per_minute"),
        config.rate_limit_per_minute,
        report,
    );
    validate_optional_positive(
        format!("{model_path}.daily_token_budget"),
        config.daily_token_budget,
        report,
    );
    validate_model_cost(format!("{model_path}.cost"), &config.cost, report);
    validate_model_profile(model_path, &provider, &config.compat_profile, report);
    validate_model_params(model_path, config, report);
    validate_model_fallbacks(model_path, &config.fallbacks, provider_settings, report);
    if provider == "mock" {
        if config.model.trim().is_empty() {
            report.warning(
                format!("{model_path}.model"),
                "mock provider is selected with an empty model name",
            );
        }
        return;
    }
    validate_required(format!("{model_path}.model"), &config.model, report);
    let effective_provider_settings = config.effective_provider_config(provider_settings);
    validate_model_provider_settings(
        &provider,
        provider_path,
        &effective_provider_settings,
        report,
    );
}

fn validate_model_fallbacks(
    model_path: &str,
    fallbacks: &[ModelFallbackConfig],
    provider_settings: &RemoteProviderConfig,
    report: &mut ConfigValidationReport,
) {
    for (index, fallback) in fallbacks.iter().enumerate() {
        let fallback_path = format!("{model_path}.fallbacks[{index}]");
        let fallback_model = fallback.model_config();
        let fallback_provider = fallback_model.effective_provider_config(provider_settings);
        validate_model_config(
            &fallback_path,
            &fallback_path,
            &fallback_model,
            &fallback_provider,
            report,
        );
    }
}

pub(super) fn validate_model_provider_settings(
    provider: &str,
    provider_path: &str,
    provider_settings: &RemoteProviderConfig,
    report: &mut ConfigValidationReport,
) {
    if provider == "ollama" {
        validate_optional_url(
            format!("{provider_path}.base_url"),
            &provider_settings.base_url,
            report,
        );
    } else {
        validate_required_url(
            format!("{provider_path}.base_url"),
            &provider_settings.base_url,
            report,
        );
        validate_required(
            format!("{provider_path}.api_key"),
            &provider_settings.api_key,
            report,
        );
    }
}

fn validate_model_profile(
    model_path: &str,
    provider: &str,
    compat_profile: &str,
    report: &mut ConfigValidationReport,
) {
    let profile = normalize(compat_profile);
    let allowed = [
        "auto",
        "generic",
        "moonshot-kimi",
        "deepseek",
        "gemini-openai",
        "openrouter",
        "qwen",
        "local-openai-compatible",
        "anthropic-native",
        "ollama-native",
        "mock",
    ];
    if !allowed.contains(&profile.as_str()) {
        report.error(
            format!("{model_path}.compat_profile"),
            "must be one of: auto, generic, moonshot-kimi, deepseek, gemini-openai, openrouter, qwen, local-openai-compatible, anthropic-native, ollama-native, mock",
        );
    }
    let allowed_for_provider = match provider {
        "openai-compatible" => !matches!(
            profile.as_str(),
            "anthropic-native" | "ollama-native" | "mock"
        ),
        "anthropic" => matches!(profile.as_str(), "auto" | "generic" | "anthropic-native"),
        "ollama" => matches!(profile.as_str(), "auto" | "generic" | "ollama-native"),
        "mock" => matches!(profile.as_str(), "auto" | "generic" | "mock"),
        _ => true,
    };
    if !allowed_for_provider {
        report.error(
            format!("{model_path}.compat_profile"),
            format!("compatibility profile `{profile}` is not valid for provider `{provider}`"),
        );
    }
}

fn validate_model_params(
    model_path: &str,
    config: &ModelConfig,
    report: &mut ConfigValidationReport,
) {
    validate_optional_positive(
        format!("{model_path}.params.max_tokens"),
        config.params.max_tokens,
        report,
    );
    validate_optional_positive(format!("{model_path}.params.n"), config.params.n, report);
    if let Some(temperature) = config.params.temperature {
        validate_float_range(
            format!("{model_path}.params.temperature"),
            temperature,
            0.0,
            2.0,
            report,
        );
    }
    if let Some(top_p) = config.params.top_p {
        validate_float_range(
            format!("{model_path}.params.top_p"),
            top_p,
            0.0,
            1.0,
            report,
        );
    }
    if let Some(presence_penalty) = config.params.presence_penalty {
        validate_float_range(
            format!("{model_path}.params.presence_penalty"),
            presence_penalty,
            -2.0,
            2.0,
            report,
        );
    }
    if let Some(frequency_penalty) = config.params.frequency_penalty {
        validate_float_range(
            format!("{model_path}.params.frequency_penalty"),
            frequency_penalty,
            -2.0,
            2.0,
            report,
        );
    }
    for (index, stop) in config.params.stop.iter().enumerate() {
        if stop.is_empty() {
            report.error(
                format!("{model_path}.params.stop[{index}]"),
                "must not be empty",
            );
        }
    }
    if let Some(effort) = &config.reasoning.effort {
        let effort = normalize(effort);
        if !matches!(
            effort.as_str(),
            "none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"
        ) {
            report.error(
                format!("{model_path}.reasoning.effort"),
                "must be one of: none, minimal, low, medium, high, xhigh, max",
            );
        }
    }
}

fn validate_model_cost(
    path: impl AsRef<str>,
    cost: &ModelCostConfig,
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    if cost.currency.trim().is_empty() {
        report.error(format!("{path}.currency"), "must not be empty");
    }
    validate_optional_cost_value(
        format!("{path}.input_per_million"),
        cost.input_per_million,
        report,
    );
    validate_optional_cost_value(
        format!("{path}.output_per_million"),
        cost.output_per_million,
        report,
    );
    validate_optional_cost_value(
        format!("{path}.cache_read_per_million"),
        cost.cache_read_per_million,
        report,
    );
    validate_optional_cost_value(
        format!("{path}.cache_write_per_million"),
        cost.cache_write_per_million,
        report,
    );
}

fn validate_optional_cost_value(
    path: impl Into<String>,
    value: Option<f64>,
    report: &mut ConfigValidationReport,
) {
    if value.is_some_and(|value| !value.is_finite() || value < 0.0) {
        report.error(
            path.into(),
            "must be a finite number greater than or equal to 0",
        );
    }
}

fn validate_model_transport(
    model_path: &str,
    provider: &str,
    transport: &str,
    report: &mut ConfigValidationReport,
) {
    let transport = transport.trim();
    if transport.is_empty() {
        report.error(format!("{model_path}.transport"), "must not be empty");
        return;
    }
    let expected = match provider {
        "mock" => "mock",
        "openai-compatible" => "openai-compatible-chat-completions",
        "anthropic" => "anthropic-messages",
        "ollama" => "ollama-chat",
        _ => return,
    };
    if transport != expected {
        report.error(
            format!("{model_path}.transport"),
            format!("provider `{provider}` requires transport `{expected}`"),
        );
    }
}
