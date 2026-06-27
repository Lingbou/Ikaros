// SPDX-License-Identifier: GPL-3.0-only

use crate::types::{ModelRequestOptions, ReasoningConfig, ReasoningEffort};
use ikaros_core::{IkarosError, ModelConfig, Result};

pub fn model_request_options_from_config(config: &ModelConfig) -> Result<ModelRequestOptions> {
    let reasoning = ReasoningConfig {
        enabled: config.reasoning.enabled,
        effort: config
            .reasoning
            .effort
            .as_deref()
            .map(parse_reasoning_effort)
            .transpose()?,
    };
    Ok(ModelRequestOptions {
        max_tokens: config.params.max_tokens,
        temperature: config.params.temperature,
        top_p: config.params.top_p,
        n: config.params.n,
        presence_penalty: config.params.presence_penalty,
        frequency_penalty: config.params.frequency_penalty,
        seed: config.params.seed,
        stop: config.params.stop.clone(),
        reasoning,
        extra_body: config.extra_body.clone(),
    })
}

fn parse_reasoning_effort(value: &str) -> Result<ReasoningEffort> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(ReasoningEffort::None),
        "minimal" => Ok(ReasoningEffort::Minimal),
        "low" => Ok(ReasoningEffort::Low),
        "medium" => Ok(ReasoningEffort::Medium),
        "high" => Ok(ReasoningEffort::High),
        "xhigh" => Ok(ReasoningEffort::XHigh),
        "max" => Ok(ReasoningEffort::Max),
        other => Err(IkarosError::Message(format!(
            "unsupported model.default.reasoning.effort `{other}`"
        ))),
    }
}

pub(crate) fn merge_request_options(
    defaults: &ModelRequestOptions,
    request: &ModelRequestOptions,
) -> ModelRequestOptions {
    let mut extra_body = defaults.extra_body.clone();
    for (key, value) in &request.extra_body {
        extra_body.insert(key.clone(), value.clone());
    }
    ModelRequestOptions {
        max_tokens: request.max_tokens.or(defaults.max_tokens),
        temperature: request.temperature.or(defaults.temperature),
        top_p: request.top_p.or(defaults.top_p),
        n: request.n.or(defaults.n),
        presence_penalty: request.presence_penalty.or(defaults.presence_penalty),
        frequency_penalty: request.frequency_penalty.or(defaults.frequency_penalty),
        seed: request.seed.or(defaults.seed),
        stop: if request.stop.is_empty() {
            defaults.stop.clone()
        } else {
            request.stop.clone()
        },
        reasoning: ReasoningConfig {
            enabled: request.reasoning.enabled.or(defaults.reasoning.enabled),
            effort: request.reasoning.effort.or(defaults.reasoning.effort),
        },
        extra_body,
    }
}
