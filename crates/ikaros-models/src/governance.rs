// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    types::{
        ModelContextProfile, ModelProvider, ModelRequest, ModelResponse, ModelStream,
        ProviderErrorKind, ProviderHealthStatus, TokenUsage,
    },
    usage::{ModelUsageLedger, ModelUsageRecord, ProviderHealthLedger, ProviderHealthRecord},
};
use async_trait::async_trait;
use ikaros_core::{IkarosError, ModelConfig, Result, now_rfc3339};
use std::time::Duration;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::time::sleep;
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
    health_ledger: ProviderHealthLedger,
    limits: ModelRuntimeLimits,
    retry_policy: ProviderRetryPolicy,
    cooldown_policy: ProviderCooldownPolicy,
}

pub struct FallbackModelProvider {
    providers: Vec<Box<dyn ModelProvider>>,
}

impl FallbackModelProvider {
    pub fn new(providers: Vec<Box<dyn ModelProvider>>) -> Result<Self> {
        if providers.is_empty() {
            return Err(IkarosError::Message(
                "fallback provider chain must contain at least one provider".into(),
            ));
        }
        Ok(Self { providers })
    }

    pub fn len(&self) -> usize {
        self.providers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderRetryPolicy {
    pub max_retries: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderCooldownPolicy {
    pub failure_threshold: u32,
    pub cooldown_ms: u64,
}

impl Default for ProviderCooldownPolicy {
    fn default() -> Self {
        Self {
            failure_threshold: 3,
            cooldown_ms: 60_000,
        }
    }
}

impl ProviderRetryPolicy {
    pub fn delay_for_attempt(&self, attempt: u32) -> u64 {
        let shift = attempt.saturating_sub(1).min(16);
        self.initial_backoff_ms
            .saturating_mul(1_u64 << shift)
            .min(self.max_backoff_ms)
    }
}

impl Default for ProviderRetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 2,
            initial_backoff_ms: 250,
            max_backoff_ms: 2_000,
        }
    }
}

impl From<&ModelConfig> for ProviderRetryPolicy {
    fn from(config: &ModelConfig) -> Self {
        Self {
            max_retries: u32::from(config.max_retries),
            ..Self::default()
        }
    }
}

impl GovernedModelProvider {
    pub fn new(
        inner: Box<dyn ModelProvider>,
        ledger: ModelUsageLedger,
        limits: ModelRuntimeLimits,
    ) -> Self {
        Self::new_with_retry_policy(inner, ledger, limits, ProviderRetryPolicy::default())
    }

    pub fn new_with_retry_policy(
        inner: Box<dyn ModelProvider>,
        ledger: ModelUsageLedger,
        limits: ModelRuntimeLimits,
        retry_policy: ProviderRetryPolicy,
    ) -> Self {
        let health_ledger = ProviderHealthLedger::for_usage_ledger(&ledger);
        Self {
            inner,
            ledger,
            health_ledger,
            limits,
            retry_policy,
            cooldown_policy: ProviderCooldownPolicy::default(),
        }
    }

    pub fn with_cooldown_policy(mut self, cooldown_policy: ProviderCooldownPolicy) -> Self {
        self.cooldown_policy = cooldown_policy;
        self
    }

    pub fn ledger(&self) -> &ModelUsageLedger {
        &self.ledger
    }

    pub fn retry_policy(&self) -> ProviderRetryPolicy {
        self.retry_policy
    }

    pub fn health_ledger(&self) -> &ProviderHealthLedger {
        &self.health_ledger
    }

    fn enforce_preflight(&self, request: &ModelRequest) -> Result<String> {
        let now = now_rfc3339()?;
        self.enforce_provider_cooldown(&now)?;
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

    fn enforce_provider_cooldown(&self, now: &str) -> Result<()> {
        let provider = self.inner.name();
        let model = provider_model_id(self.inner.as_ref());
        if let Some(record) = self.health_ledger.latest(provider, model)?
            && let Some(cooldown_until) = record.cooldown_until
            && cooldown_until.as_str() > now
        {
            return Err(IkarosError::Message(format!(
                "provider {provider}/{model} is in cooldown until {cooldown_until}"
            )));
        }
        Ok(())
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

    fn record_provider_success(&self, at: &str, provider: &str, model: &str) -> Result<()> {
        self.health_ledger.append(ProviderHealthRecord {
            at: at.into(),
            provider: provider.into(),
            model: model.into(),
            status: ProviderHealthStatus::Healthy,
            consecutive_failures: 0,
            last_error_kind: None,
            last_error_summary: String::new(),
            cooldown_until: None,
        })
    }

    fn record_provider_failure(
        &self,
        at: &str,
        kind: ProviderErrorKind,
        summary: &str,
    ) -> Result<()> {
        let provider = self.inner.name();
        let model = provider_model_id(self.inner.as_ref());
        let previous_failures = self
            .health_ledger
            .latest(provider, model)?
            .filter(|record| record.status != ProviderHealthStatus::Healthy)
            .map(|record| record.consecutive_failures)
            .unwrap_or_default();
        let consecutive_failures = previous_failures.saturating_add(1);
        let unavailable = consecutive_failures >= self.cooldown_policy.failure_threshold;
        self.health_ledger.append(ProviderHealthRecord {
            at: at.into(),
            provider: provider.into(),
            model: model.into(),
            status: if unavailable {
                ProviderHealthStatus::Unavailable
            } else {
                ProviderHealthStatus::Degraded
            },
            consecutive_failures,
            last_error_kind: Some(kind),
            last_error_summary: summary.into(),
            cooldown_until: unavailable
                .then(|| cooldown_until_rfc3339(self.cooldown_policy.cooldown_ms))
                .transpose()?,
        })
    }
}

fn provider_model_id(provider: &dyn ModelProvider) -> &str {
    let model = provider.model_id();
    if model.trim().is_empty() {
        "unknown"
    } else {
        model
    }
}

fn cooldown_until_rfc3339(cooldown_ms: u64) -> Result<String> {
    let duration = time::Duration::milliseconds(cooldown_ms.min(i64::MAX as u64) as i64);
    Ok((OffsetDateTime::now_utc() + duration).format(&Rfc3339)?)
}

fn classify_provider_error(error: &IkarosError) -> ProviderErrorKind {
    let text = error.to_string().to_ascii_lowercase();
    if text.contains("429") || text.contains("rate limit") || text.contains("rate_limit") {
        ProviderErrorKind::RateLimited
    } else if text.contains("503")
        || text.contains("502")
        || text.contains("500")
        || text.contains("timeout")
        || text.contains("timed out")
        || text.contains("connection reset")
    {
        ProviderErrorKind::Transient
    } else if text.contains("401") || text.contains("403") || text.contains("auth") {
        ProviderErrorKind::Auth
    } else if text.contains("context length")
        || text.contains("context window")
        || text.contains("too many tokens")
    {
        ProviderErrorKind::ContextLimit
    } else if text.contains("400") || text.contains("bad request") || text.contains("schema") {
        ProviderErrorKind::BadRequest
    } else if text.contains("network") || text.contains("dns") {
        ProviderErrorKind::Network
    } else {
        ProviderErrorKind::Unknown
    }
}

#[async_trait]
impl ModelProvider for GovernedModelProvider {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
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
        let mut attempt = 0;
        loop {
            match self.inner.generate(request.clone()).await {
                Ok(response) => {
                    self.record_provider_success(
                        &requested_at,
                        &response.provider,
                        &response.model,
                    )?;
                    self.record_usage(
                        requested_at,
                        estimate,
                        &response.provider,
                        &response.model,
                        &response.usage,
                    )?;
                    return Ok(response);
                }
                Err(error) => {
                    let kind = classify_provider_error(&error);
                    if attempt >= self.retry_policy.max_retries || !kind.retryable() {
                        self.record_provider_failure(&requested_at, kind, &error.to_string())?;
                        return Err(error);
                    }
                    attempt += 1;
                    let delay = self.retry_policy.delay_for_attempt(attempt);
                    if delay > 0 {
                        sleep(Duration::from_millis(delay)).await;
                    }
                }
            }
        }
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let request = request.redacted();
        let requested_at = self.enforce_preflight(&request)?;
        let estimate = self.inner.estimate_request_tokens(&request);
        let mut attempt = 0;
        loop {
            match self.inner.stream(request.clone()).await {
                Ok(stream) => {
                    self.record_provider_success(&requested_at, &stream.provider, &stream.model)?;
                    self.record_usage(
                        requested_at,
                        estimate,
                        &stream.provider,
                        &stream.model,
                        &stream.usage,
                    )?;
                    return Ok(stream);
                }
                Err(error) => {
                    let kind = classify_provider_error(&error);
                    if attempt >= self.retry_policy.max_retries || !kind.retryable() {
                        self.record_provider_failure(&requested_at, kind, &error.to_string())?;
                        return Err(error);
                    }
                    attempt += 1;
                    let delay = self.retry_policy.delay_for_attempt(attempt);
                    if delay > 0 {
                        sleep(Duration::from_millis(delay)).await;
                    }
                }
            }
        }
    }
}

#[async_trait]
impl ModelProvider for FallbackModelProvider {
    fn name(&self) -> &str {
        "fallback-chain"
    }

    fn model_id(&self) -> &str {
        self.providers
            .first()
            .map(|provider| provider.model_id())
            .unwrap_or_default()
    }

    fn estimate_request_tokens(&self, request: &ModelRequest) -> u32 {
        self.providers
            .first()
            .map(|provider| provider.estimate_request_tokens(request))
            .unwrap_or_else(|| request.estimated_tokens())
    }

    fn context_profile(&self) -> ModelContextProfile {
        self.providers
            .first()
            .map(|provider| provider.context_profile())
            .unwrap_or_default()
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let mut last_error = None;
        for provider in &self.providers {
            match provider.generate(request.clone()).await {
                Ok(response) => return Ok(response),
                Err(error) => {
                    let kind = classify_provider_error(&error);
                    if !kind.retryable() {
                        return Err(error);
                    }
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            IkarosError::Message("fallback provider chain failed without an error".into())
        }))
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let mut last_error = None;
        for provider in &self.providers {
            match provider.stream(request.clone()).await {
                Ok(stream) => return Ok(stream),
                Err(error) => {
                    let kind = classify_provider_error(&error);
                    if !kind.retryable() {
                        return Err(error);
                    }
                    last_error = Some(error);
                }
            }
        }
        Err(last_error.unwrap_or_else(|| {
            IkarosError::Message("fallback provider chain failed without an error".into())
        }))
    }
}
