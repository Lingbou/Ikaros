// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_host::{WorkbenchModelBudgetStatus, WorkbenchProviderStatusReport};
use ikaros_providers::model::ModelUsageLedger;
use std::path::Path;

use super::super::{path_display, terminal_inline};
use super::provider::{
    active_provider_status_report, format_model_budget_status, format_model_cost_status,
    format_model_fallback_status, model_budget_json,
};
use super::queue::continuation_count;

pub(super) fn print_unified_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    _usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    let provider = active_provider_status_report(paths, runtime)?;
    let approvals_pending = runtime.session.pending_approvals()?.len();
    let continuations = continuation_count(config, paths, workspace, runtime)?;
    println!(
        "status_model: provider={} model={} profile={} profile_source={} context_window={} default_output_tokens={} tokenizer={} runtime={} transport={} health={} fallback_count={}",
        terminal_inline(&provider.provider),
        terminal_inline(&provider.model),
        terminal_inline(&provider.provider_profile),
        provider.profile_source,
        terminal_inline(&provider.context_window),
        terminal_inline(&provider.default_output_tokens),
        terminal_inline(&provider.tokenizer),
        terminal_inline(&provider.runtime),
        terminal_inline(&provider.transport),
        terminal_inline(&provider.health.status),
        provider.fallback_count
    );
    println!(
        "status_model_policy: temperature={} reasoning={} message={} tool_schema={} request_body={} prompt_cache={} retry_without_parameters={}",
        terminal_inline(&provider.temperature_policy),
        terminal_inline(&provider.reasoning_policy),
        terminal_inline(&provider.message_policy),
        terminal_inline(&provider.tool_schema_policy),
        terminal_inline(&provider.request_body_policy),
        terminal_inline(&provider.prompt_cache_policy),
        terminal_inline(&provider.retry_without_parameters),
    );
    println!(
        "status_model_budget: {}",
        format_model_budget_status(&provider.budget)
    );
    println!(
        "status_model_cost: {}",
        format_model_cost_status(&provider.cost)
    );
    println!(
        "status_model_fallbacks: {}",
        format_model_fallback_status(&provider)
    );
    println!("status_workspace: {}", path_display(workspace));
    println!(
        "status_policy: workspace_writes={} shell={} network={}",
        runtime.agent.profile.workspace_writes,
        runtime.agent.profile.shell,
        runtime.agent.profile.network
    );
    println!("status_approvals_pending: {approvals_pending}");
    println!("status_continuations: {continuations}");
    println!(
        "{}",
        workbench_status_json_line(WorkbenchStatusJsonInput {
            provider: &provider,
            runtime,
            workspace,
            approvals_pending,
            continuations,
        })?
    );
    Ok(())
}

pub(super) fn unified_status_human_lines(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    _usage_ledger: &ModelUsageLedger,
) -> Result<Vec<String>> {
    let provider = active_provider_status_report(paths, runtime)?;
    let approvals_pending = runtime.session.pending_approvals()?.len();
    let continuations = continuation_count(config, paths, workspace, runtime)?;

    Ok(vec![
        "* Status".to_owned(),
        format!("  workspace: {}", path_display(workspace)),
        format!(
            "  model: {} ({})",
            terminal_inline(&provider.model),
            terminal_inline(&provider.provider)
        ),
        format!(
            "  profile: {} ({})",
            terminal_inline(&provider.provider_profile),
            provider.profile_source
        ),
        format!("  health: {}", terminal_inline(&provider.health.status)),
        format!("  budget: {}", human_model_budget_status(&provider.budget)),
        "  permissions:".to_owned(),
        format!(
            "    workspace writes: {}",
            runtime.agent.profile.workspace_writes
        ),
        format!("    shell: {}", runtime.agent.profile.shell),
        format!("    network: {}", runtime.agent.profile.network),
        "  pending:".to_owned(),
        format!("    approvals: {approvals_pending}"),
        format!("    queued continuations: {continuations}"),
    ])
}

fn human_model_budget_status(budget: &WorkbenchModelBudgetStatus) -> String {
    match budget.daily_token_budget {
        Some(limit) => {
            format!(
                "{} used today, {} remaining of {}",
                budget.used_today,
                budget.remaining_today.unwrap_or_default(),
                limit
            )
        }
        None => format!("{} used today, no daily limit", budget.used_today),
    }
}

struct WorkbenchStatusJsonInput<'a> {
    provider: &'a WorkbenchProviderStatusReport,
    runtime: &'a InteractiveChatRuntime,
    workspace: &'a Path,
    approvals_pending: usize,
    continuations: usize,
}

fn workbench_status_json_line(input: WorkbenchStatusJsonInput<'_>) -> Result<String> {
    let payload = serde_json::json!({
        "schema": "ikaros-workbench-status-v1",
        "version": 1,
        "session": {
            "session_id": terminal_inline(&input.runtime.chat_session_id),
            "state_db": terminal_inline(&input.runtime.state_dir.join("state.db").display().to_string()),
        },
        "agent": {
            "id": terminal_inline(&input.runtime.agent_id),
            "profile": terminal_inline(&input.runtime.agent.name),
            "mode": input.runtime.agent.mode().to_string(),
            "policy": {
                "workspace_writes": input.runtime.agent.profile.workspace_writes.to_string(),
                "shell": input.runtime.agent.profile.shell.to_string(),
                "network": input.runtime.agent.profile.network.to_string(),
            },
        },
        "workspace": terminal_inline(&input.workspace.display().to_string()),
        "model": {
            "provider": terminal_inline(&input.provider.provider),
            "model": terminal_inline(&input.provider.model),
            "profile": terminal_inline(&input.provider.provider_profile),
            "profile_source": input.provider.profile_source,
            "context_window": terminal_inline(&input.provider.context_window),
            "default_output_tokens": terminal_inline(&input.provider.default_output_tokens),
            "tokenizer": terminal_inline(&input.provider.tokenizer),
            "runtime": terminal_inline(&input.provider.runtime),
            "transport": terminal_inline(&input.provider.transport),
            "health": terminal_inline(&input.provider.health.status),
            "fallback_count": input.provider.fallback_count,
            "budget": model_budget_json(&input.provider.budget),
        },
        "counts": {
            "approvals_pending": input.approvals_pending,
            "continuations": input.continuations,
        },
        "actions": {
            "screen": "/screen",
            "timeline": "/timeline",
            "trace": "/trace",
            "provider_debug": "/provider debug",
            "provider_matrix": "/provider matrix",
            "approval": "/approval",
            "cancel_all": "/cancel all",
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-status-v1","version":1,"error":"serialization_failed"}"#
            .to_owned()
    });
    Ok(format!("workbench_status_json: {encoded}"))
}
