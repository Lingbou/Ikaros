// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{Result, redact_secrets};
use ikaros_providers::model::{ModelProviderDescriptor, ModelUsageLedger, ModelUsageRecord};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProviderMatrixUsageSummary {
    pub(super) requests: usize,
    pub(super) prompt_tokens: u64,
    pub(super) completion_tokens: u64,
    pub(super) total_tokens: u64,
    pub(super) cache_read_tokens: u64,
    pub(super) cache_write_tokens: u64,
    pub(super) estimated_cost_today: String,
    pub(super) cache_accounting: &'static str,
}

pub(super) fn provider_matrix_usage_summary(
    usage: &ModelUsageLedger,
    provider: &str,
    model: &str,
    descriptor: Option<&ModelProviderDescriptor>,
) -> Result<ProviderMatrixUsageSummary> {
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let records = usage.read_all()?;
    let today_records = records
        .iter()
        .filter(|record| {
            record.at.starts_with(&today) && record.provider == provider && record.model == model
        })
        .collect::<Vec<_>>();
    let prompt_tokens = today_records
        .iter()
        .map(|record| record.prompt_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let completion_tokens = today_records
        .iter()
        .map(|record| record.completion_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let total_tokens = today_records
        .iter()
        .map(|record| record.total_tokens as u64)
        .sum::<u64>();
    let cache_read_tokens = today_records
        .iter()
        .map(|record| record.cache_read_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let cache_write_tokens = today_records
        .iter()
        .map(|record| record.cache_write_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let cost = descriptor.map(|descriptor| &descriptor.cost);
    let estimated_cost_today = cost
        .and_then(|cost| provider_debug_estimated_cost_today(&today_records, cost))
        .unwrap_or_else(|| "unknown".into());
    let cache_accounting = cost
        .map(provider_debug_cache_accounting)
        .unwrap_or("unavailable");
    Ok(ProviderMatrixUsageSummary {
        requests: today_records.len(),
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cache_read_tokens,
        cache_write_tokens,
        estimated_cost_today,
        cache_accounting,
    })
}
pub(super) fn provider_debug_usage_summary(
    usage: &ModelUsageLedger,
    provider: &str,
    model: &str,
    descriptor: Option<&ModelProviderDescriptor>,
) -> Result<Value> {
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let records = usage.read_all()?;
    let today_records = records
        .iter()
        .filter(|record| {
            record.at.starts_with(&today) && record.provider == provider && record.model == model
        })
        .collect::<Vec<_>>();
    let prompt_tokens = today_records
        .iter()
        .map(|record| record.prompt_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let completion_tokens = today_records
        .iter()
        .map(|record| record.completion_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let total_tokens = today_records
        .iter()
        .map(|record| record.total_tokens as u64)
        .sum::<u64>();
    let cache_read_tokens = today_records
        .iter()
        .map(|record| record.cache_read_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let cache_write_tokens = today_records
        .iter()
        .map(|record| record.cache_write_tokens.unwrap_or_default() as u64)
        .sum::<u64>();
    let estimated_records = today_records
        .iter()
        .filter(|record| record.estimated)
        .count();
    let cost = descriptor.map(|descriptor| &descriptor.cost);
    let estimated_cost_today =
        cost.and_then(|cost| provider_debug_estimated_cost_today(&today_records, cost));
    Ok(json!({
        "day": today,
        "requests": today_records.len(),
        "estimated_records": estimated_records,
        "prompt_tokens": prompt_tokens,
        "completion_tokens": completion_tokens,
        "total_tokens": total_tokens,
        "cache_read_tokens": cache_read_tokens,
        "cache_write_tokens": cache_write_tokens,
        "currency": cost.map(|cost| redact_secrets(&cost.currency)),
        "estimated_cost_today": estimated_cost_today,
        "cache_accounting": cost
            .map(provider_debug_cache_accounting)
            .unwrap_or("unavailable"),
    }))
}

fn provider_debug_estimated_cost_today(
    records: &[&ModelUsageRecord],
    cost: &ikaros_providers::model::ModelProviderCost,
) -> Option<String> {
    let (Some(input), Some(output)) = (cost.input_per_million, cost.output_per_million) else {
        return None;
    };
    let prompt_tokens = records
        .iter()
        .map(|record| record.prompt_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let completion_tokens = records
        .iter()
        .map(|record| record.completion_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let cache_read_tokens = records
        .iter()
        .map(|record| record.cache_read_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let cache_write_tokens = records
        .iter()
        .map(|record| record.cache_write_tokens.unwrap_or_default() as f64)
        .sum::<f64>();
    let cache_read_price = cost.cache_read_per_million.unwrap_or(input);
    let cache_write_price = cost.cache_write_per_million.unwrap_or(input);
    let regular_input_tokens =
        (prompt_tokens - cache_read_tokens - cache_write_tokens).clamp(0.0, f64::MAX);
    Some(format!(
        "{:.6}",
        ((regular_input_tokens * input)
            + (completion_tokens * output)
            + (cache_read_tokens * cache_read_price)
            + (cache_write_tokens * cache_write_price))
            / 1_000_000.0
    ))
}

fn provider_debug_cache_accounting(
    cost: &ikaros_providers::model::ModelProviderCost,
) -> &'static str {
    if cost.cache_read_per_million.is_some() || cost.cache_write_per_million.is_some() {
        "priced"
    } else if cost.input_per_million.is_some() || cost.output_per_million.is_some() {
        "tracked"
    } else {
        "unavailable"
    }
}
