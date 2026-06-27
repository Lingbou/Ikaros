// SPDX-License-Identifier: GPL-3.0-only

use super::policy::{ModelRuntimeLimits, ProviderCooldownPolicy, provider_model_id};
use crate::{
    types::{ModelProvider, ModelRequest, ProviderErrorKind, ProviderHealthStatus, TokenUsage},
    usage::{ModelUsageLedger, ModelUsageRecord, ProviderHealthLedger, ProviderHealthRecord},
};
use ikaros_core::{IkarosError, Result, now_rfc3339};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

pub(super) fn enforce_preflight(
    inner: &dyn ModelProvider,
    ledger: &ModelUsageLedger,
    health_ledger: &ProviderHealthLedger,
    limits: &ModelRuntimeLimits,
    cooldown_policy: &ProviderCooldownPolicy,
    request: &ModelRequest,
) -> Result<String> {
    let now = now_rfc3339()?;
    enforce_provider_cooldown(inner, health_ledger, cooldown_policy, &now)?;
    if let Some(limit) = limits.rate_limit_per_minute {
        let minute = now.get(..16).unwrap_or(&now);
        let count = ledger.requests_for_minute(minute)?;
        if count >= limit as usize {
            return Err(IkarosError::Message(format!(
                "model rate limit exceeded: {count}/{limit} requests in the current minute"
            )));
        }
    }
    if let Some(budget) = limits.daily_token_budget {
        let day = now.get(..10).unwrap_or(&now);
        let used = ledger.total_for_day(day)?;
        let estimate = inner.estimate_request_tokens(request);
        if used.saturating_add(estimate) > budget {
            return Err(IkarosError::Message(format!(
                "model daily token budget exceeded: used {used}, estimated request {estimate}, budget {budget}; raise or disable model.default.daily_token_budget in config.yaml"
            )));
        }
    }
    Ok(now)
}

fn enforce_provider_cooldown(
    inner: &dyn ModelProvider,
    health_ledger: &ProviderHealthLedger,
    _cooldown_policy: &ProviderCooldownPolicy,
    now: &str,
) -> Result<()> {
    let provider = inner.name();
    let model = provider_model_id(inner);
    if let Some(record) = health_ledger.latest(provider, model)?
        && let Some(cooldown_until) = record.cooldown_until
        && cooldown_until.as_str() > now
    {
        return Err(IkarosError::Message(format!(
            "provider {provider}/{model} is in cooldown until {cooldown_until}"
        )));
    }
    Ok(())
}

pub(super) fn record_usage(
    ledger: &ModelUsageLedger,
    requested_at: String,
    estimate: u32,
    provider: &str,
    model: &str,
    usage: &TokenUsage,
) -> Result<()> {
    let usage_total = usage.total_or_prompt_completion();
    let estimated = usage_total == 0;
    ledger.append(ModelUsageRecord {
        id: Uuid::new_v4().to_string(),
        at: requested_at,
        provider: provider.into(),
        model: model.into(),
        prompt_tokens: usage.prompt_tokens,
        completion_tokens: usage.completion_tokens,
        total_tokens: if estimated { estimate } else { usage_total },
        cache_read_tokens: usage.cache_read_tokens,
        cache_write_tokens: usage.cache_write_tokens,
        estimated,
    })
}

pub(super) fn record_provider_success(
    health_ledger: &ProviderHealthLedger,
    at: &str,
    provider: &str,
    model: &str,
) -> Result<()> {
    health_ledger.append(ProviderHealthRecord {
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

pub(super) fn record_provider_failure(
    inner: &dyn ModelProvider,
    health_ledger: &ProviderHealthLedger,
    cooldown_policy: &ProviderCooldownPolicy,
    at: &str,
    kind: ProviderErrorKind,
    summary: &str,
) -> Result<()> {
    let provider = inner.name();
    let model = provider_model_id(inner);
    let previous_failures = health_ledger
        .latest(provider, model)?
        .filter(|record| record.status != ProviderHealthStatus::Healthy)
        .map(|record| record.consecutive_failures)
        .unwrap_or_default();
    let consecutive_failures = previous_failures.saturating_add(1);
    let unavailable = consecutive_failures >= cooldown_policy.failure_threshold;
    health_ledger.append(ProviderHealthRecord {
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
            .then(|| cooldown_until_rfc3339(cooldown_policy.cooldown_ms))
            .transpose()?,
    })
}

fn cooldown_until_rfc3339(cooldown_ms: u64) -> Result<String> {
    let duration = time::Duration::milliseconds(cooldown_ms.min(i64::MAX as u64) as i64);
    Ok((OffsetDateTime::now_utc() + duration).format(&Rfc3339)?)
}
