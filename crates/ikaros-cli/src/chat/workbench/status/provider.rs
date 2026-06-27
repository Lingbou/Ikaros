// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::IkarosPaths;
use ikaros_host::{
    WorkbenchModelBudgetStatus, WorkbenchModelCostStatus, WorkbenchProviderHealthStatus,
    WorkbenchProviderStatusReport, model_budget_status_report, workbench_provider_status_report,
};

use super::super::{WorkbenchCell, WorkbenchCellKind, terminal_inline};

pub(in crate::chat) fn active_provider_status_report(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<WorkbenchProviderStatusReport> {
    Ok(workbench_provider_status_report(
        paths,
        &runtime.model_config,
        &runtime.model_provider,
    )?)
}

pub(in crate::chat) fn active_model_budget_status(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<WorkbenchModelBudgetStatus> {
    Ok(model_budget_status_report(paths, &runtime.model_config)?)
}

pub(in crate::chat) fn format_model_budget_status(budget: &WorkbenchModelBudgetStatus) -> String {
    match budget.daily_token_budget {
        Some(limit) => format!(
            "daily_token_budget={limit} used_today={} remaining_today={} budget_status={}",
            budget.used_today,
            budget.remaining_today.unwrap_or_default(),
            budget.budget_status
        ),
        None => format!(
            "daily_token_budget=disabled used_today={} remaining_today=unbounded budget_status={}",
            budget.used_today, budget.budget_status
        ),
    }
}

pub(super) fn model_budget_json(budget: &WorkbenchModelBudgetStatus) -> serde_json::Value {
    serde_json::json!({
        "daily_token_budget": budget.daily_token_budget,
        "used_today": budget.used_today,
        "remaining_today": budget.remaining_today,
        "budget_status": budget.budget_status,
    })
}

pub(super) fn format_model_cost_status(cost: &WorkbenchModelCostStatus) -> String {
    format!(
        "currency={} input_per_million={} output_per_million={} cache_read_per_million={} cache_write_per_million={} estimated_cost_today={} cache_read_tokens_today={} cache_write_tokens_today={} cache_accounting={}",
        terminal_inline(&cost.currency),
        terminal_inline(&cost.input_per_million),
        terminal_inline(&cost.output_per_million),
        terminal_inline(&cost.cache_read_per_million),
        terminal_inline(&cost.cache_write_per_million),
        terminal_inline(&cost.estimated_cost_today),
        cost.cache_read_tokens_today,
        cost.cache_write_tokens_today,
        cost.cache_accounting
    )
}

pub(super) fn format_model_fallback_status(report: &WorkbenchProviderStatusReport) -> String {
    format!(
        "fallback_count={} fallback_chain={}",
        report.fallback_count,
        terminal_inline(&report.fallback_chain)
    )
}

pub(in crate::chat) fn print_model_status(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let report = active_provider_status_report(paths, runtime)?;
    println!("model_source: active_runtime");
    println!("provider: {}", terminal_inline(&report.provider));
    println!("model: {}", terminal_inline(&report.model));
    println!(
        "configured_profile: {}",
        terminal_inline(&report.configured_profile)
    );
    println!("profile: {}", terminal_inline(&report.provider_profile));
    println!("profile_source: {}", report.profile_source);
    println!(
        "temperature_policy: {}",
        terminal_inline(&report.temperature_policy)
    );
    println!(
        "reasoning_policy: {}",
        terminal_inline(&report.reasoning_policy)
    );
    println!(
        "message_policy: {}",
        terminal_inline(&report.message_policy)
    );
    println!(
        "tool_schema_policy: {}",
        terminal_inline(&report.tool_schema_policy)
    );
    println!(
        "request_body_policy: {}",
        terminal_inline(&report.request_body_policy)
    );
    println!(
        "prompt_cache_policy: {}",
        terminal_inline(&report.prompt_cache_policy)
    );
    println!(
        "retry_without_parameters: {}",
        terminal_inline(&report.retry_without_parameters)
    );
    print_model_fallback_rows(&report);
    println!(
        "context_window: {}",
        terminal_inline(&report.context_window)
    );
    println!(
        "default_output_tokens: {}",
        terminal_inline(&report.default_output_tokens)
    );
    println!("tokenizer: {}", terminal_inline(&report.tokenizer));
    println!("runtime: {}", terminal_inline(&report.runtime));
    println!("transport: {}", terminal_inline(&report.transport));
    println!("streaming: {}", report.streaming);
    println!("tool_calls: {}", report.tool_calls);
    println!("reasoning: {}", report.reasoning);
    println!("json_mode: {}", report.json_mode);
    println!("network: {}", report.network);
    println!("image_input: {}", report.image_input);
    println!("audio_input: {}", report.audio_input);
    println!("file_input: {}", report.file_input);
    println!("health: {}", terminal_inline(&report.health.status));
    Ok(())
}

pub(in crate::chat) fn print_model_status_for_human(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    for line in model_status_human_lines(paths, runtime)? {
        println!("{line}");
    }
    Ok(())
}

pub(in crate::chat) fn model_status_human_lines(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
) -> Result<Vec<String>> {
    let report = active_provider_status_report(paths, runtime)?;
    Ok(vec![
        "* Model".to_owned(),
        format!("  name: {}", terminal_inline(&report.model)),
        format!("  provider: {}", terminal_inline(&report.provider)),
        format!(
            "  profile: {} ({})",
            terminal_inline(&report.provider_profile),
            report.profile_source
        ),
        format!("  health: {}", terminal_inline(&report.health.status)),
        "  capabilities:".to_owned(),
        format!("    streaming: {}", report.streaming),
        format!("    tools: {}", report.tool_calls),
        format!("    reasoning: {}", report.reasoning),
        format!("    image: {}", report.image_input),
        format!("    audio: {}", report.audio_input),
        format!("    file: {}", report.file_input),
        format!("  transport: {}", terminal_inline(&report.transport)),
        "  switch: edit config for now; /model is inspect-only.".to_owned(),
    ])
}

pub(in crate::chat) fn print_provider_status_for_human(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
    args: &[&str],
) -> Result<()> {
    for line in provider_status_human_lines(paths, runtime, args)? {
        println!("{line}");
    }
    Ok(())
}

pub(in crate::chat) fn provider_status_human_lines(
    paths: &IkarosPaths,
    runtime: &InteractiveChatRuntime,
    args: &[&str],
) -> Result<Vec<String>> {
    let report = active_provider_status_report(paths, runtime)?;
    let mut lines = vec!["* Provider".to_owned()];
    if !args.is_empty() {
        lines.push(format!("  view: {}", terminal_inline(&args.join(" "))));
    }
    lines.push(format!("  provider: {}", terminal_inline(&report.provider)));
    lines.push(format!("  model: {}", terminal_inline(&report.model)));
    lines.push(format!(
        "  profile: {} ({})",
        terminal_inline(&report.provider_profile),
        report.profile_source
    ));
    lines.push(format!(
        "  health: {}",
        terminal_inline(&report.health.status)
    ));
    lines.push(format!("  streaming: {}", report.streaming));
    lines.push(format!("  tools: {}", report.tool_calls));
    lines.push(format!("  reasoning: {}", report.reasoning));
    lines.push(format!(
        "  context window: {}",
        terminal_inline(&report.context_window)
    ));
    lines.push(format!(
        "  transport: {}",
        terminal_inline(&report.transport)
    ));
    if args.iter().any(|arg| *arg == "--live") {
        lines.push(
            "  live probe: use `ikaros provider health --live` for detailed probe output"
                .to_owned(),
        );
    }
    Ok(lines)
}

fn print_model_fallback_rows(report: &WorkbenchProviderStatusReport) {
    println!("fallback_count: {}", report.fallback_count);
    for fallback in &report.fallbacks {
        println!(
            "fallback_row: index={} provider={} model={} configured_profile={} profile={} live_smoke={} streaming={} tool_calls={} reasoning={} network={} image_input={} audio_input={} file_input={} context_window={} default_output_tokens={}",
            fallback.index,
            terminal_inline(&fallback.provider),
            terminal_inline(&fallback.model),
            terminal_inline(&fallback.configured_profile),
            terminal_inline(&fallback.provider_profile),
            fallback.live_smoke,
            fallback.streaming,
            fallback.tool_calls,
            fallback.reasoning,
            fallback.network,
            fallback.image_input,
            fallback.audio_input,
            fallback.file_input,
            terminal_inline(&fallback.context_window),
            terminal_inline(&fallback.default_output_tokens),
        );
    }
}

fn model_input_capabilities(report: &WorkbenchProviderStatusReport) -> String {
    format!(
        "image={} audio={} file={}",
        report.image_input, report.audio_input, report.file_input
    )
}

pub(super) fn screen_provider_cells(report: &WorkbenchProviderStatusReport) -> Vec<WorkbenchCell> {
    let mut cells = vec![
        WorkbenchCell {
            kind: WorkbenchCellKind::Model,
            title: "provider matrix".into(),
            detail: format!(
                "provider={} model={} profile={} source={} context_window={} default_output_tokens={} tokenizer={} input_capabilities=\"{}\" command=/provider matrix matrix=/provider matrix live=/provider matrix --live health=/provider health debug=/provider debug inspect=/provider inspect",
                terminal_inline(&report.provider),
                terminal_inline(&report.model),
                terminal_inline(&report.provider_profile),
                report.profile_source,
                terminal_inline(&report.context_window),
                terminal_inline(&report.default_output_tokens),
                terminal_inline(&report.tokenizer),
                model_input_capabilities(report),
            ),
        },
        WorkbenchCell {
            kind: WorkbenchCellKind::Model,
            title: "provider cost".into(),
            detail: format!(
                "{} command=/provider debug matrix=/provider matrix debug=/provider debug",
                format_model_cost_status(&report.cost)
            ),
        },
        screen_model_budget_cell(&report.budget),
        screen_provider_cache_policy_cell(report),
        screen_provider_health_cell(&report.health),
        screen_provider_recovery_cell(report),
        WorkbenchCell {
            kind: WorkbenchCellKind::Model,
            title: "provider fallback".into(),
            detail: format!(
                "{} command=/provider debug debug=/provider debug inspect=/provider inspect",
                format_model_fallback_status(report)
            ),
        },
    ];
    cells.extend(screen_provider_fallback_cells(report));
    cells
}

fn screen_provider_cache_policy_cell(report: &WorkbenchProviderStatusReport) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "provider cache policy".into(),
        detail: format!(
            "prompt_cache_policy={} retry_without_parameters={} request_body_policy={} tool_schema_policy={} context=/context debug=/provider debug matrix=/provider matrix",
            terminal_inline(&report.prompt_cache_policy),
            terminal_inline(&report.retry_without_parameters),
            terminal_inline(&report.request_body_policy),
            terminal_inline(&report.tool_schema_policy),
        ),
    }
}

fn screen_provider_recovery_cell(report: &WorkbenchProviderStatusReport) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "provider recovery".into(),
        detail: format!(
            "status={} last_error_kind={} cooldown_until={} fallback_count={} streaming={} tool_calls={} health=/provider health live=/provider health --live matrix=/provider matrix --live fallback=/provider matrix debug=/provider debug trace=/trace --kind model budget=/budget disable_budget=/budget disable",
            terminal_inline(&report.health.status),
            terminal_inline(&report.health.last_error_kind),
            terminal_inline(&report.health.cooldown_until),
            report.fallback_count,
            report.streaming,
            report.tool_calls,
        ),
    }
}

fn screen_provider_fallback_cells(report: &WorkbenchProviderStatusReport) -> Vec<WorkbenchCell> {
    report
        .fallbacks
        .iter()
        .map(|fallback| WorkbenchCell {
            kind: WorkbenchCellKind::Model,
            title: format!("fallback {}", fallback.index + 1),
            detail: format!(
                "provider={} model={} profile={} live_smoke={} context_window={} default_output_tokens={} matrix=/provider matrix debug=/provider debug health=/provider health",
                terminal_inline(&fallback.provider),
                terminal_inline(&fallback.model),
                terminal_inline(&fallback.provider_profile),
                fallback.live_smoke,
                terminal_inline(&fallback.context_window),
                terminal_inline(&fallback.default_output_tokens),
            ),
        })
        .collect()
}

fn screen_model_budget_cell(budget: &WorkbenchModelBudgetStatus) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "model budget".into(),
        detail: format!(
            "{} command=/budget raise=/budget set {} disable=/budget disable",
            format_model_budget_status(budget),
            budget.suggested_daily_token_budget
        ),
    }
}

pub(super) fn screen_provider_health_cell(health: &WorkbenchProviderHealthStatus) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: "provider health".into(),
        detail: format!(
            "health_status={} consecutive_failures={} last_error_kind={} cooldown_until={} last_error_summary={} command=/provider health health=/provider health live=/provider health --live",
            terminal_inline(&health.status),
            health.consecutive_failures,
            terminal_inline(&health.last_error_kind),
            terminal_inline(&health.cooldown_until),
            terminal_inline(&health.last_error_summary),
        ),
    }
}
