// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosPaths, ModelCostConfig};
use ikaros_models::{
    ModelProviderDescriptor, ModelUsageLedger, ModelUsageRecord, ProviderHealthLedger,
    ProviderRegistry,
};

use super::super::{WorkbenchCell, WorkbenchCellKind, terminal_inline};

pub(in crate::chat) fn format_model_budget_status(
    model: &ikaros_core::ModelConfig,
    usage_ledger: &ModelUsageLedger,
) -> Result<String> {
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let used = usage_ledger.total_for_day(&today)?;
    Ok(match model.daily_token_budget {
        Some(budget) => {
            let remaining = budget.saturating_sub(used);
            let status = if used >= budget {
                "exhausted"
            } else if used.saturating_mul(10) >= budget.saturating_mul(9) {
                "near_limit"
            } else {
                "ok"
            };
            format!(
                "daily_token_budget={budget} used_today={used} remaining_today={remaining} budget_status={status}"
            )
        }
        None => format!(
            "daily_token_budget=disabled used_today={used} remaining_today=unbounded budget_status=unbounded"
        ),
    })
}

pub(super) fn model_budget_json(
    model: &ikaros_core::ModelConfig,
    usage_ledger: &ModelUsageLedger,
) -> Result<serde_json::Value> {
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let used = usage_ledger.total_for_day(&today)?;
    Ok(match model.daily_token_budget {
        Some(budget) => {
            let remaining = budget.saturating_sub(used);
            let status = if used >= budget {
                "exhausted"
            } else if used.saturating_mul(10) >= budget.saturating_mul(9) {
                "near_limit"
            } else {
                "ok"
            };
            serde_json::json!({
                "daily_token_budget": budget,
                "used_today": used,
                "remaining_today": remaining,
                "budget_status": status,
            })
        }
        None => serde_json::json!({
            "daily_token_budget": null,
            "used_today": used,
            "remaining_today": null,
            "budget_status": "unbounded",
        }),
    })
}

pub(super) fn format_model_cost_status(
    descriptor: Option<&ModelProviderDescriptor>,
    usage_ledger: &ModelUsageLedger,
) -> Result<String> {
    let Some(descriptor) = descriptor else {
        return Ok(
            "currency=unknown input_per_million=unknown output_per_million=unknown cache_read_per_million=unknown cache_write_per_million=unknown estimated_cost_today=unknown cache_accounting=unavailable"
                .into(),
        );
    };
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let records = usage_ledger.read_all()?;
    let today_records = records
        .iter()
        .filter(|record| record.at.starts_with(&today))
        .collect::<Vec<_>>();
    let cache_read_tokens = today_records
        .iter()
        .map(|record| record.cache_read_tokens.unwrap_or_default())
        .sum::<u32>();
    let cache_write_tokens = today_records
        .iter()
        .map(|record| record.cache_write_tokens.unwrap_or_default())
        .sum::<u32>();
    let estimated_cost = estimated_model_cost_today(&today_records, &descriptor.cost);
    let cache_accounting = if descriptor.cost.cache_read_per_million.is_some()
        || descriptor.cost.cache_write_per_million.is_some()
    {
        "priced"
    } else {
        "tracked"
    };
    Ok(format!(
        "currency={} input_per_million={} output_per_million={} cache_read_per_million={} cache_write_per_million={} estimated_cost_today={} cache_read_tokens_today={} cache_write_tokens_today={} cache_accounting={}",
        terminal_inline(&descriptor.cost.currency),
        optional_cost(descriptor.cost.input_per_million),
        optional_cost(descriptor.cost.output_per_million),
        optional_cost(descriptor.cost.cache_read_per_million),
        optional_cost(descriptor.cost.cache_write_per_million),
        estimated_cost,
        cache_read_tokens,
        cache_write_tokens,
        cache_accounting
    ))
}

fn estimated_model_cost_today(
    records: &[&ModelUsageRecord],
    cost: &ikaros_models::ModelProviderCost,
) -> String {
    let (Some(input), Some(output)) = (cost.input_per_million, cost.output_per_million) else {
        return "unknown".into();
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
    format!(
        "{:.6}",
        ((regular_input_tokens * input)
            + (completion_tokens * output)
            + (cache_read_tokens * cache_read_price)
            + (cache_write_tokens * cache_write_price))
            / 1_000_000.0
    )
}

fn optional_cost(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.4}"))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn format_model_fallback_status(model: &ikaros_core::ModelConfig) -> String {
    if model.fallbacks.is_empty() {
        return "fallback_count=0 fallback_chain=none".into();
    }
    let chain = model
        .fallbacks
        .iter()
        .enumerate()
        .map(|(index, fallback)| {
            format!(
                "{}:{}/{} profile={}",
                index,
                terminal_inline(&fallback.provider),
                terminal_inline(&fallback.model),
                terminal_inline(&fallback.compat_profile)
            )
        })
        .collect::<Vec<_>>()
        .join(" -> ");
    format!(
        "fallback_count={} fallback_chain={}",
        model.fallbacks.len(),
        chain
    )
}

pub(in crate::chat) fn print_model_status(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let model = &runtime.model_config;
    let descriptor = ProviderRegistry
        .descriptor_with_profile(
            &model.provider,
            &runtime.model_provider.base_url,
            &model.model,
            &model.compat_profile,
        )
        .ok();
    let provider_health = ProviderHealthLedger::new(&paths.audit_dir)
        .latest(&model.provider, &model.model)?
        .map(|record| format!("{:?}", record.status))
        .unwrap_or_else(|| "Unknown".into());
    println!("model_source: active_runtime");
    println!("provider: {}", terminal_inline(&model.provider));
    println!("model: {}", terminal_inline(&model.model));
    println!(
        "configured_profile: {}",
        terminal_inline(&model.compat_profile)
    );
    println!("profile: {}", model_profile(&descriptor));
    println!(
        "profile_source: {}",
        model_profile_source(&model.provider, &model.compat_profile, descriptor.as_ref())
    );
    println!(
        "temperature_policy: {}",
        model_policy(&descriptor, |policy| &policy.temperature)
    );
    println!(
        "reasoning_policy: {}",
        model_policy(&descriptor, |policy| &policy.reasoning)
    );
    println!(
        "message_policy: {}",
        model_policy(&descriptor, |policy| &policy.message)
    );
    println!(
        "tool_schema_policy: {}",
        model_policy(&descriptor, |policy| &policy.tool_schema)
    );
    println!(
        "request_body_policy: {}",
        model_policy(&descriptor, |policy| &policy.request_body)
    );
    println!(
        "prompt_cache_policy: {}",
        model_policy(&descriptor, |policy| &policy.prompt_cache)
    );
    println!(
        "retry_without_parameters: {}",
        model_retry_without_parameters(&descriptor)
    );
    print_model_fallback_rows(model)?;
    println!("context_window: {}", model_context_window(&descriptor));
    println!(
        "default_output_tokens: {}",
        model_default_output_tokens(&descriptor)
    );
    println!("tokenizer: {}", model_tokenizer(&descriptor));
    println!("runtime: {}", terminal_inline(&model.runtime));
    println!("transport: {}", terminal_inline(&model.transport));
    println!(
        "streaming: {}",
        model_capability(&descriptor, |descriptor| {
            descriptor.capabilities.streaming
        })
    );
    println!(
        "tool_calls: {}",
        model_capability(&descriptor, |descriptor| {
            descriptor.capabilities.tool_calls
        })
    );
    println!(
        "reasoning: {}",
        model_capability(&descriptor, |descriptor| {
            descriptor.capabilities.reasoning
        })
    );
    println!(
        "json_mode: {}",
        model_capability(&descriptor, |descriptor| {
            descriptor.capabilities.json_mode
        })
    );
    println!(
        "network: {}",
        model_capability(&descriptor, |descriptor| {
            descriptor.capabilities.network
        })
    );
    println!(
        "image_input: {}",
        model_capability(&descriptor, |descriptor| {
            descriptor.capabilities.image_input
        })
    );
    println!(
        "audio_input: {}",
        model_capability(&descriptor, |descriptor| {
            descriptor.capabilities.audio_input
        })
    );
    println!(
        "file_input: {}",
        model_capability(&descriptor, |descriptor| {
            descriptor.capabilities.file_input
        })
    );
    println!("health: {}", terminal_inline(&provider_health));
    Ok(())
}

fn print_model_fallback_rows(model: &ikaros_core::ModelConfig) -> Result<()> {
    let registry = ProviderRegistry;
    println!("fallback_count: {}", model.fallbacks.len());
    for (index, fallback) in model.fallbacks.iter().enumerate() {
        let fallback_model = fallback.model_config();
        let fallback_provider = fallback.provider_config();
        let descriptor = registry.descriptor_with_profile(
            &fallback_model.provider,
            &fallback_provider.base_url,
            &fallback_model.model,
            &fallback_model.compat_profile,
        )?;
        println!(
            "fallback_row: index={} provider={} model={} configured_profile={} profile={} live_smoke={} streaming={} tool_calls={} reasoning={} network={} image_input={} audio_input={} file_input={} context_window={} default_output_tokens={}",
            index,
            terminal_inline(&descriptor.provider),
            terminal_inline(&descriptor.model),
            terminal_inline(&fallback_model.compat_profile),
            terminal_inline(&descriptor.profile),
            model_fallback_live_smoke_state(
                &fallback_model.provider,
                &fallback_model.model,
                !fallback_provider.base_url.trim().is_empty(),
                !fallback_provider.api_key.trim().is_empty(),
            ),
            descriptor.capabilities.streaming,
            descriptor.capabilities.tool_calls,
            descriptor.capabilities.reasoning,
            descriptor.capabilities.network,
            descriptor.capabilities.image_input,
            descriptor.capabilities.audio_input,
            descriptor.capabilities.file_input,
            descriptor.context.context_window,
            descriptor.context.default_output_tokens,
        );
    }
    Ok(())
}

fn model_fallback_live_smoke_state(
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

pub(super) fn model_profile(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| terminal_inline(&descriptor.profile))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn model_profile_source(
    provider: &str,
    configured_profile: &str,
    descriptor: Option<&ModelProviderDescriptor>,
) -> &'static str {
    if provider.trim().eq_ignore_ascii_case("openai-compatible") {
        let configured = configured_profile.trim().to_ascii_lowercase();
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

pub(super) fn model_context_window(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| descriptor.context.context_window.to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn model_default_output_tokens(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| descriptor.context.default_output_tokens.to_string())
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn model_tokenizer(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| format!("{:?}", descriptor.context.tokenizer))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn model_policy(
    descriptor: &Option<ModelProviderDescriptor>,
    read: impl FnOnce(&ikaros_models::ModelProviderProfilePolicy) -> &str,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| terminal_inline(read(&descriptor.profile_policy)))
        .unwrap_or_else(|| "unknown".into())
}

pub(super) fn model_retry_without_parameters(
    descriptor: &Option<ModelProviderDescriptor>,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| {
            if descriptor
                .profile_policy
                .retry_without_parameters
                .is_empty()
            {
                "none".into()
            } else {
                terminal_inline(&descriptor.profile_policy.retry_without_parameters.join(","))
            }
        })
        .unwrap_or_else(|| "unknown".into())
}

fn model_capability(
    descriptor: &Option<ModelProviderDescriptor>,
    read: impl FnOnce(&ModelProviderDescriptor) -> bool,
) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| read(descriptor).to_string())
        .unwrap_or_else(|| "unknown".into())
}

fn model_input_capabilities(descriptor: &Option<ModelProviderDescriptor>) -> String {
    descriptor
        .as_ref()
        .map(|descriptor| {
            format!(
                "image={} audio={} file={}",
                descriptor.capabilities.image_input,
                descriptor.capabilities.audio_input,
                descriptor.capabilities.file_input
            )
        })
        .unwrap_or_else(|| "image=unknown audio=unknown file=unknown".into())
}

pub(super) fn screen_provider_cells(
    model: &ikaros_core::ModelConfig,
    descriptor: Option<&ModelProviderDescriptor>,
    usage_ledger: &ModelUsageLedger,
    health_ledger: &ProviderHealthLedger,
) -> Result<Vec<WorkbenchCell>> {
    let descriptor_owned = descriptor.cloned();
    let health = health_ledger.latest(&model.provider, &model.model)?;
    let mut cells = vec![
        WorkbenchCell {
            kind: WorkbenchCellKind::Model,
            title: "provider matrix".into(),
            detail: format!(
                "provider={} model={} profile={} source={} context_window={} default_output_tokens={} tokenizer={} input_capabilities=\"{}\" command=/provider matrix matrix=/provider matrix live=/provider matrix --live health=/provider health debug=/provider debug inspect=/provider inspect",
                terminal_inline(&model.provider),
                terminal_inline(&model.model),
                model_profile(&descriptor_owned),
                model_profile_source(&model.provider, &model.compat_profile, descriptor),
                model_context_window(&descriptor_owned),
                model_default_output_tokens(&descriptor_owned),
                model_tokenizer(&descriptor_owned),
                model_input_capabilities(&descriptor_owned),
            ),
        },
        WorkbenchCell {
            kind: WorkbenchCellKind::Model,
            title: "provider cost".into(),
            detail: format!(
                "{} command=/provider debug matrix=/provider matrix debug=/provider debug",
                format_model_cost_status(descriptor, usage_ledger)?
            ),
        },
        screen_model_budget_cell(model, usage_ledger)?,
        screen_provider_cache_policy_cell(&descriptor_owned),
        screen_provider_health_cell(health.as_ref()),
        screen_provider_recovery_cell(model, &descriptor_owned, health.as_ref()),
        WorkbenchCell {
            kind: WorkbenchCellKind::Model,
            title: "provider fallback".into(),
            detail: format!(
                "{} command=/provider debug debug=/provider debug inspect=/provider inspect",
                format_model_fallback_status(model)
            ),
        },
    ];
    cells.extend(screen_provider_fallback_cells(model));
    Ok(cells)
}

fn screen_provider_cache_policy_cell(
    descriptor: &Option<ModelProviderDescriptor>,
) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "provider cache policy".into(),
        detail: format!(
            "prompt_cache_policy={} retry_without_parameters={} request_body_policy={} tool_schema_policy={} context=/context debug=/provider debug matrix=/provider matrix",
            model_policy(descriptor, |policy| &policy.prompt_cache),
            model_retry_without_parameters(descriptor),
            model_policy(descriptor, |policy| &policy.request_body),
            model_policy(descriptor, |policy| &policy.tool_schema),
        ),
    }
}

fn screen_provider_recovery_cell(
    model: &ikaros_core::ModelConfig,
    descriptor: &Option<ModelProviderDescriptor>,
    health: Option<&ikaros_models::ProviderHealthRecord>,
) -> WorkbenchCell {
    let cooldown_until = health
        .and_then(|record| record.cooldown_until.as_deref())
        .unwrap_or_else(|| "none".into());
    let last_error_kind = health
        .and_then(|record| record.last_error_kind)
        .map(|kind| format!("{kind:?}"))
        .unwrap_or_else(|| "none".into());
    let status = health
        .map(|record| format!("{:?}", record.status))
        .unwrap_or_else(|| "Unknown".into());
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "provider recovery".into(),
        detail: format!(
            "status={} last_error_kind={} cooldown_until={} fallback_count={} streaming={} tool_calls={} health=/provider health live=/provider health --live matrix=/provider matrix --live fallback=/provider matrix debug=/provider debug trace=/trace --kind model budget=/budget disable_budget=/budget disable",
            terminal_inline(&status),
            terminal_inline(&last_error_kind),
            terminal_inline(cooldown_until),
            model.fallbacks.len(),
            model_capability(descriptor, |descriptor| descriptor.capabilities.streaming),
            model_capability(descriptor, |descriptor| descriptor.capabilities.tool_calls),
        ),
    }
}

fn screen_provider_fallback_cells(model: &ikaros_core::ModelConfig) -> Vec<WorkbenchCell> {
    let registry = ProviderRegistry;
    model
        .fallbacks
        .iter()
        .enumerate()
        .map(|(index, fallback)| {
            let fallback_model = fallback.model_config();
            let fallback_provider = fallback.provider_config();
            let descriptor = registry
                .descriptor_with_profile(
                    &fallback_model.provider,
                    &fallback_provider.base_url,
                    &fallback_model.model,
                    &fallback_model.compat_profile,
                )
                .ok();
            WorkbenchCell {
                kind: WorkbenchCellKind::Model,
                title: format!("fallback {}", index + 1),
                detail: format!(
                    "provider={} model={} profile={} live_smoke={} context_window={} default_output_tokens={} matrix=/provider matrix debug=/provider debug health=/provider health",
                    terminal_inline(&fallback_model.provider),
                    terminal_inline(&fallback_model.model),
                    model_profile(&descriptor),
                    model_fallback_live_smoke_state(
                        &fallback_model.provider,
                        &fallback_model.model,
                        !fallback_provider.base_url.trim().is_empty(),
                        !fallback_provider.api_key.trim().is_empty(),
                    ),
                    model_context_window(&descriptor),
                    model_default_output_tokens(&descriptor),
                ),
            }
        })
        .collect()
}

fn screen_model_budget_cell(
    model: &ikaros_core::ModelConfig,
    usage_ledger: &ModelUsageLedger,
) -> Result<WorkbenchCell> {
    let today = time::OffsetDateTime::now_utc().date().to_string();
    let used = usage_ledger.total_for_day(&today)?;
    let suggested = suggested_daily_token_budget(model.daily_token_budget, used);
    Ok(WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "model budget".into(),
        detail: format!(
            "{} command=/budget raise=/budget set {} disable=/budget disable",
            format_model_budget_status(model, usage_ledger)?,
            suggested
        ),
    })
}

fn suggested_daily_token_budget(current: Option<u32>, used: u32) -> u32 {
    let usage_headroom = used.saturating_add(100_000);
    let configured_headroom = current
        .map(|budget| budget.saturating_mul(2))
        .unwrap_or(100_000);
    usage_headroom.max(configured_headroom).max(100_000)
}

pub(super) fn apply_configured_model_cost(
    descriptor: &mut ModelProviderDescriptor,
    cost: &ModelCostConfig,
) {
    if !model_cost_is_configured(cost) {
        return;
    }
    descriptor.cost.currency = terminal_inline(&cost.currency);
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

fn model_cost_is_configured(cost: &ModelCostConfig) -> bool {
    cost.currency.trim() != "USD"
        || cost.input_per_million.is_some()
        || cost.output_per_million.is_some()
        || cost.cache_read_per_million.is_some()
        || cost.cache_write_per_million.is_some()
}

pub(super) fn screen_provider_health_cell(
    health: Option<&ikaros_models::ProviderHealthRecord>,
) -> WorkbenchCell {
    let Some(health) = health else {
        return WorkbenchCell {
            kind: WorkbenchCellKind::Model,
            title: "provider health".into(),
            detail:
                "health_status=Unknown consecutive_failures=0 last_error_kind=none cooldown_until=none command=/provider health health=/provider health live=/provider health --live"
                    .into(),
        };
    };
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "provider health".into(),
        detail: format!(
            "health_status={:?} consecutive_failures={} last_error_kind={} cooldown_until={} last_error_summary={} command=/provider health health=/provider health live=/provider health --live",
            health.status,
            health.consecutive_failures,
            health
                .last_error_kind
                .map(|kind| format!("{kind:?}"))
                .unwrap_or_else(|| "none".into()),
            health.cooldown_until.as_deref().unwrap_or("none"),
            terminal_inline(&health.last_error_summary),
        ),
    }
}
