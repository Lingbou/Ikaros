// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::{
    InteractiveChatRuntime, InteractiveChatStatusInput, format_interactive_chat_status,
};
use anyhow::Result;
use ikaros_agent::body_status::base_body_status;
use ikaros_agent::chat::ChatRunOptions;
use ikaros_core::{IkarosConfig, IkarosPaths};
#[cfg(test)]
use ikaros_execution::harness::{ApprovalRecord, ApprovalStatus};
use ikaros_host::session_state_db_candidates_with_config;
#[cfg(test)]
use ikaros_protocol::{GatewayMessageKind, GatewayRoute};
use ikaros_providers::model::ModelUsageLedger;
use ikaros_state::automation::LocalScheduleStore;
#[cfg(test)]
use ikaros_state::gateway::LocalGatewayStore;
#[cfg(test)]
use ikaros_state::session::SessionReplay;
use ikaros_state::session::{AgentEventKind, SessionId, SessionStore, SqliteSessionStore};
#[cfg(test)]
use std::collections::VecDeque;
#[cfg(test)]
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(test)]
use super::{WorkbenchCell, WorkbenchScreen};
use super::{agent_event_cell, path_display};

mod api;
mod approval;
mod context;
mod diff;
mod gateway;
mod memory;
mod provider;
mod queue;
mod screen;
mod session;
mod timeline;
mod tools;
mod unified;

pub(in crate::chat) use api::{
    api_status_human_lines, print_api_status, print_api_status_for_human,
};
#[cfg(test)]
use approval::approval_overlay_json_line;
pub(in crate::chat) use approval::print_approval_status;
#[cfg(test)]
use approval::screen_approval_cells;
#[cfg(test)]
use context::screen_context_cells_from_replay;
pub(in crate::chat) use context::{
    context_status_human_lines, print_context_status, print_context_status_for_human,
};
pub(in crate::chat) use diff::{print_diff_status, print_diff_status_for_human};
#[cfg(test)]
use gateway::screen_gateway_status_cell;
pub(in crate::chat) use gateway::{
    gateway_status_human_lines, print_gateway_status, print_gateway_status_for_human,
};
#[cfg(test)]
use ikaros_terminal::screen_progress_status_cell;
pub(in crate::chat) use memory::{
    memory_status_human_lines, print_memory_status, print_memory_status_for_human,
};
#[cfg(test)]
use provider::screen_provider_cells;
pub(in crate::chat) use provider::{
    active_model_budget_status, format_model_budget_status, model_status_human_lines,
    print_model_status, print_model_status_for_human, print_provider_status_for_human,
    provider_status_human_lines,
};
#[cfg(test)]
use provider::{format_model_cost_status, format_model_fallback_status};
#[cfg(test)]
use queue::{screen_queue_status_cell, screen_side_cells};
pub(in crate::chat) use screen::{print_screen_status_with_state, selected_screen_primary_action};
pub(in crate::chat) use session::{
    print_session_export, print_session_history, print_session_status, print_session_summaries,
    session_history_human_lines, session_status_human_lines, session_summaries_human_lines,
};
pub(in crate::chat) use timeline::{
    TimelineRequest, TimelineVerbosity, print_replay_status, print_replay_status_for_human,
    print_trace_status, print_trace_status_for_human,
};
#[cfg(test)]
use timeline::{
    screen_coding_cells_from_replay, screen_failure_cells_from_replay,
    screen_timeline_cells_from_replay, timeline_point_matches,
};
pub(in crate::chat) use tools::{
    mcp_status_human_lines, print_mcp_status, print_mcp_status_for_human, print_rag_status,
    print_rag_status_for_human, print_tools_status, print_tools_status_for_human,
    rag_status_human_lines, tools_status_human_lines,
};
use unified::{print_unified_status, unified_status_human_lines};

pub(in crate::chat) fn print_workbench_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    print_session_status(config, paths, workspace, runtime, options)?;
    print_unified_status(config, paths, workspace, runtime, usage_ledger)?;
    let body_status = base_body_status(paths)?;
    println!(
        "{}",
        format_interactive_chat_status(InteractiveChatStatusInput {
            agent: &runtime.agent,
            session: &runtime.session,
            chat_session_id: &runtime.chat_session_id,
            state_dir: &runtime.state_dir,
            options,
            emotion: &body_status.emotion,
            usage_ledger,
        })
    );
    Ok(())
}

pub(in crate::chat) fn print_workbench_status_for_human(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    for line in
        workbench_status_human_lines(config, paths, workspace, runtime, options, usage_ledger)?
    {
        println!("{line}");
    }
    Ok(())
}

pub(in crate::chat) fn workbench_status_human_lines(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<Vec<String>> {
    let mut lines = session_status_human_lines(config, paths, workspace, runtime, options)?;
    lines.extend(unified_status_human_lines(
        config,
        paths,
        workspace,
        runtime,
        usage_ledger,
    )?);
    Ok(lines)
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    let mut truncated = String::new();
    for (index, ch) in input.chars().enumerate() {
        if index >= max_chars {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
    }
    truncated
}

fn state_db_candidates(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<Vec<PathBuf>> {
    Ok(session_state_db_candidates_with_config(
        paths,
        config,
        workspace,
        Some(&runtime.agent.name),
        true,
        false,
    )?)
}

fn task_counts(paths: &IkarosPaths) -> Result<(PathBuf, usize, usize, usize, usize)> {
    let store = LocalScheduleStore::new(&paths.automation_dir);
    let jobs = store.list()?;
    let due = store.due_now()?;
    let enabled = jobs.iter().filter(|job| job.enabled).count();
    let disabled = jobs.len().saturating_sub(enabled);
    Ok((
        store.path().to_path_buf(),
        jobs.len(),
        enabled,
        disabled,
        due.len(),
    ))
}

pub(in crate::chat) fn print_tasks_status(paths: &IkarosPaths) -> Result<()> {
    let (store_path, total, enabled, disabled, due) = task_counts(paths)?;
    println!("tasks_store: {}", path_display(&store_path));
    println!("tasks_total: {total}");
    println!("tasks_enabled: {enabled}");
    println!("tasks_disabled: {disabled}");
    println!("tasks_due: {due}");
    Ok(())
}

pub(in crate::chat) fn tasks_status_human_lines(paths: &IkarosPaths) -> Result<Vec<String>> {
    let (store_path, total, enabled, disabled, due) = task_counts(paths)?;
    Ok(vec![
        "* Tasks".to_owned(),
        format!("  store: {}", path_display(&store_path)),
        format!("  total: {total}"),
        format!("  enabled: {enabled}"),
        format!("  disabled: {disabled}"),
        format!("  due now: {due}"),
    ])
}

fn print_filtered_event_cells(
    runtime: &InteractiveChatRuntime,
    label: &str,
    filter: impl Fn(&AgentEventKind) -> bool,
) -> Result<()> {
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let Some(replay) = store.replay_session(&session_id)? else {
        println!("{label}_timeline_events: 0");
        return Ok(());
    };
    let events = replay
        .agent_events
        .iter()
        .filter(|event| filter(&event.kind))
        .collect::<Vec<_>>();
    println!("{label}_timeline_events: {}", events.len());
    let start = events.len().saturating_sub(5);
    for event in &events[start..] {
        println!("- {}", agent_event_cell(event).render());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
