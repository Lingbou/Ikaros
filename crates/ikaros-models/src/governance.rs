// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    types::{
        ModelContextProfile, ModelProvider, ModelRequest, ModelResponse, ModelStream, TokenUsage,
    },
    usage::{ModelUsageLedger, ModelUsageRecord},
};
use async_trait::async_trait;
use ikaros_core::{IkarosError, ModelConfig, Result, now_rfc3339};
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct ModelRuntimeLimits {
    pub rate_limit_per_minute: Option<u32>,
    pub daily_token_budget: Option<u32>,
}

impl From<&ModelConfig> for ModelRuntimeLimits {
    fn from(config: &ModelConfig) -> Self {
        Self {
            rate_limit_per_minute: config.rate_limit_per_minute,
            daily_token_budget: config.daily_token_budget,
        }
    }
}

pub struct GovernedModelProvider {
    inner: Box<dyn ModelProvider>,
    ledger: ModelUsageLedger,
    limits: ModelRuntimeLimits,
}

impl GovernedModelProvider {
    pub fn new(
        inner: Box<dyn ModelProvider>,
        ledger: ModelUsageLedger,
        limits: ModelRuntimeLimits,
    ) -> Self {
        Self {
            inner,
            ledger,
            limits,
        }
    }

    pub fn ledger(&self) -> &ModelUsageLedger {
        &self.ledger
    }

    fn enforce_preflight(&self, request: &ModelRequest) -> Result<String> {
        let now = now_rfc3339()?;
        if let Some(limit) = self.limits.rate_limit_per_minute {
            let minute = now.get(..16).unwrap_or(&now);
            let count = self.ledger.requests_for_minute(minute)?;
            if count >= limit as usize {
                return Err(IkarosError::Message(format!(
                    "model rate limit exceeded: {count}/{limit} requests in the current minute"
                )));
            }
        }
        if let Some(budget) = self.limits.daily_token_budget {
            let day = now.get(..10).unwrap_or(&now);
            let used = self.ledger.total_for_day(day)?;
            let estimate = self.inner.estimate_request_tokens(request);
            if used.saturating_add(estimate) > budget {
                return Err(IkarosError::Message(format!(
                    "model daily token budget exceeded: used {used}, estimated request {estimate}, budget {budget}"
                )));
            }
        }
        Ok(now)
    }

    fn record_usage(
        &self,
        requested_at: String,
        estimate: u32,
        provider: &str,
        model: &str,
        usage: &TokenUsage,
    ) -> Result<()> {
        let usage_total = usage.total_or_prompt_completion();
        let estimated = usage_total == 0;
        self.ledger.append(ModelUsageRecord {
            id: Uuid::new_v4().to_string(),
            at: requested_at,
            provider: provider.into(),
            model: model.into(),
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            total_tokens: if estimated { estimate } else { usage_total },
            estimated,
        })
    }
}

#[async_trait]
impl ModelProvider for GovernedModelProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn estimate_request_tokens(&self, request: &ModelRequest) -> u32 {
        self.inner.estimate_request_tokens(request)
    }

    fn context_profile(&self) -> ModelContextProfile {
        self.inner.context_profile()
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let request = request.redacted();
        let requested_at = self.enforce_preflight(&request)?;
        let estimate = self.inner.estimate_request_tokens(&request);
        let response = self.inner.generate(request).await?;
        self.record_usage(
            requested_at,
            estimate,
            &response.provider,
            &response.model,
            &response.usage,
        )?;
        Ok(response)
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let request = request.redacted();
        let requested_at = self.enforce_preflight(&request)?;
        let estimate = self.inner.estimate_request_tokens(&request);
        let stream = self.inner.stream(request).await?;
        self.record_usage(
            requested_at,
            estimate,
            &stream.provider,
            &stream.model,
            &stream.usage,
        )?;
        Ok(stream)
    }
}
