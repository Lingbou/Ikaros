// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

use super::{ModelProviderKind, ModelTransportKind};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelTable {
    pub default: ModelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelConfig {
    pub preset: Option<String>,
    pub provider: ModelProviderKind,
    pub runtime: String,
    pub transport: ModelTransportKind,
    pub model: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    pub compat_profile: String,
    pub params: ModelParamsConfig,
    pub reasoning: ModelReasoningConfig,
    pub extra_body: serde_json::Map<String, serde_json::Value>,
    pub cost: ModelCostConfig,
    pub timeout_ms: u64,
    pub max_retries: u8,
    pub rate_limit_per_minute: Option<u32>,
    pub daily_token_budget: Option<u32>,
    pub fallbacks: Vec<ModelFallbackConfig>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            preset: None,
            provider: ModelProviderKind::OpenaiCompatible,
            runtime: "harness-agent-loop".into(),
            transport: ModelTransportKind::OpenaiCompatibleChatCompletions,
            model: String::new(),
            api_key: None,
            base_url: None,
            compat_profile: "auto".into(),
            params: ModelParamsConfig::default(),
            reasoning: ModelReasoningConfig::default(),
            extra_body: serde_json::Map::new(),
            cost: ModelCostConfig::default(),
            timeout_ms: 30_000,
            max_retries: 0,
            rate_limit_per_minute: None,
            daily_token_budget: None,
            fallbacks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelFallbackConfig {
    pub preset: Option<String>,
    pub provider: ModelProviderKind,
    pub runtime: String,
    pub transport: ModelTransportKind,
    pub model: String,
    pub compat_profile: String,
    pub params: ModelParamsConfig,
    pub reasoning: ModelReasoningConfig,
    pub extra_body: serde_json::Map<String, serde_json::Value>,
    pub cost: ModelCostConfig,
    pub timeout_ms: u64,
    pub max_retries: u8,
    pub rate_limit_per_minute: Option<u32>,
    pub daily_token_budget: Option<u32>,
    pub api_key: String,
    pub base_url: String,
}

impl Default for ModelFallbackConfig {
    fn default() -> Self {
        let model = ModelConfig::default();
        Self {
            preset: None,
            provider: model.provider,
            runtime: model.runtime,
            transport: model.transport,
            model: model.model,
            api_key: String::new(),
            base_url: String::new(),
            compat_profile: model.compat_profile,
            params: model.params,
            reasoning: model.reasoning,
            extra_body: model.extra_body,
            cost: model.cost,
            timeout_ms: model.timeout_ms,
            max_retries: model.max_retries,
            rate_limit_per_minute: model.rate_limit_per_minute,
            daily_token_budget: model.daily_token_budget,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelCostConfig {
    pub currency: String,
    pub input_per_million: Option<f64>,
    pub output_per_million: Option<f64>,
    pub cache_read_per_million: Option<f64>,
    pub cache_write_per_million: Option<f64>,
}

impl Default for ModelCostConfig {
    fn default() -> Self {
        Self {
            currency: "USD".into(),
            input_per_million: None,
            output_per_million: None,
            cache_read_per_million: None,
            cache_write_per_million: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelParamsConfig {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub n: Option<u32>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub seed: Option<u64>,
    pub stop: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ModelReasoningConfig {
    pub enabled: Option<bool>,
    pub effort: Option<String>,
}
