// SPDX-License-Identifier: GPL-3.0-only

use crate::types::{ModelProvider, ProviderErrorKind};
use ikaros_core::{IkarosError, ModelConfig};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderRetryPolicy {
    pub max_retries: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub jitter_ms: u64,
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
            jitter_ms: 100,
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

pub(super) fn provider_model_id(provider: &dyn ModelProvider) -> &str {
    let model = provider.model_id();
    if model.trim().is_empty() {
        "unknown"
    } else {
        model
    }
}

pub(super) fn classify_provider_error(error: &IkarosError) -> ProviderErrorKind {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ProviderRetryDelay {
    pub(super) delay_ms: u64,
    pub(super) base_delay_ms: u64,
    pub(super) jitter_ms: u64,
    pub(super) retry_after_ms: Option<u64>,
}

pub(super) fn retry_delay_for_error(
    policy: &ProviderRetryPolicy,
    attempt: u32,
    error: &IkarosError,
) -> ProviderRetryDelay {
    let retry_after_ms = retry_after_ms_from_error(error);
    let base_delay_ms = retry_after_ms.unwrap_or_else(|| policy.delay_for_attempt(attempt));
    let jitter_ms = jitter_ms_for_retry(policy);
    ProviderRetryDelay {
        delay_ms: base_delay_ms.saturating_add(jitter_ms),
        base_delay_ms,
        jitter_ms,
        retry_after_ms,
    }
}

fn jitter_ms_for_retry(policy: &ProviderRetryPolicy) -> u64 {
    let max_jitter = policy.jitter_ms.min(60_000);
    if max_jitter == 0 {
        return 0;
    }
    (Uuid::new_v4().as_u128() % (u128::from(max_jitter) + 1)) as u64
}

fn retry_after_ms_from_error(error: &IkarosError) -> Option<u64> {
    let text = error.to_string();
    for marker in ["retry-after", "retry_after", "retry after"] {
        if let Some(value) = retry_after_value_after_marker(&text, marker) {
            return Some(value);
        }
    }
    None
}

fn retry_after_value_after_marker(text: &str, marker: &str) -> Option<u64> {
    let lower = text.to_ascii_lowercase();
    let start = lower.find(marker)?.saturating_add(marker.len());
    let mut value = text[start..]
        .trim_start_matches(|ch: char| ch == ':' || ch == '=' || ch.is_whitespace())
        .chars()
        .take_while(|ch| ch.is_ascii_digit() || *ch == '.')
        .collect::<String>();
    if value.ends_with('.') {
        value.pop();
    }
    if value.is_empty() {
        return None;
    }
    let seconds = value.parse::<f64>().ok()?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some((seconds * 1000.0).ceil().min(u64::MAX as f64) as u64)
}

pub(super) fn provider_error_kind_label(kind: ProviderErrorKind) -> &'static str {
    match kind {
        ProviderErrorKind::Auth => "auth",
        ProviderErrorKind::RateLimited => "rate_limited",
        ProviderErrorKind::Transient => "transient",
        ProviderErrorKind::BadRequest => "bad_request",
        ProviderErrorKind::ContextLimit => "context_limit",
        ProviderErrorKind::Network => "network",
        ProviderErrorKind::Unknown => "unknown",
    }
}
