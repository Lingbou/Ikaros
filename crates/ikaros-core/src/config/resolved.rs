// SPDX-License-Identifier: GPL-3.0-only

use super::{IkarosConfig, ModelConfig, ModelFallbackConfig, RemoteProviderConfig};

impl IkarosConfig {
    pub fn effective_model_provider(&self) -> RemoteProviderConfig {
        self.model
            .default
            .effective_provider_config(&self.providers.model)
    }
}

impl ModelFallbackConfig {
    pub fn model_config(&self) -> ModelConfig {
        ModelConfig {
            provider: self.provider,
            runtime: self.runtime.clone(),
            transport: self.transport,
            model: self.model.clone(),
            api_key: non_empty_string(&self.api_key),
            base_url: non_empty_string(&self.base_url),
            compat_profile: self.compat_profile.clone(),
            params: self.params.clone(),
            reasoning: self.reasoning.clone(),
            extra_body: self.extra_body.clone(),
            cost: self.cost.clone(),
            preset: self.preset.clone(),
            timeout_ms: self.timeout_ms,
            max_retries: self.max_retries,
            rate_limit_per_minute: self.rate_limit_per_minute,
            daily_token_budget: self.daily_token_budget,
            fallbacks: Vec::new(),
        }
    }

    pub fn provider_config(&self) -> RemoteProviderConfig {
        RemoteProviderConfig {
            api_key: non_empty_string(&self.api_key).unwrap_or_default(),
            base_url: non_empty_string(&self.base_url).unwrap_or_default(),
        }
    }
}

impl ModelConfig {
    pub fn effective_provider_config(
        &self,
        fallback: &RemoteProviderConfig,
    ) -> RemoteProviderConfig {
        RemoteProviderConfig {
            api_key: optional_non_empty(&self.api_key).unwrap_or_else(|| fallback.api_key.clone()),
            base_url: optional_non_empty(&self.base_url)
                .unwrap_or_else(|| fallback.base_url.clone()),
        }
    }
}

fn optional_non_empty(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn non_empty_string(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_owned())
    }
}
