// SPDX-License-Identifier: GPL-3.0-only

mod input;
mod mentions;
mod slash;
mod status;

use anyhow::Result;
use ikaros_core::{IkarosPaths, redact_secrets};
use ikaros_models::ModelStreamEvent;
use ikaros_session::{AgentEvent, AgentEventKind, SessionEntry};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

pub(super) use super::output::render_terminal_markdown;
#[cfg(test)]
pub(super) use ikaros_tui::render_workbench_snapshot;
pub(super) use ikaros_tui::{WorkbenchCell, WorkbenchCellKind, terminal_inline, terminal_message};

pub(super) use super::tui::{
    WorkbenchScreen, WorkbenchScreenApprovalAction, WorkbenchScreenContinuationAction,
    WorkbenchScreenInputAction, WorkbenchScreenOpenAction, WorkbenchScreenState,
    apply_workbench_screen_args, command_requires_explicit_action,
    render_fullscreen_terminal_frame, render_fullscreen_workbench_with_state, screen_json_line,
    screen_selected_actions_json_line, screen_selected_actions_line, screen_selected_cell_line,
    screen_selected_primary_action,
};
pub(super) use input::{
    WorkbenchInputAction, WorkbenchInputEvent, WorkbenchInputState, WorkbenchTerminalInputOutcome,
    apply_workbench_terminal_input_event, format_workbench_input_state,
    parse_workbench_input_event, parse_workbench_terminal_event,
};
pub(super) use mentions::{context_mentions_human_lines, print_context_mentions};
pub(super) use slash::{
    SlashCommandPaletteItem, format_slash_command_help, print_slash_commands,
    print_slash_commands_for_human, slash_command_palette_items, slash_command_palette_summary,
    slash_command_registry_summary, slash_commands_human_lines, suggest_slash_command,
};
pub(super) use status::{
    TimelineRequest, TimelineVerbosity, api_status_human_lines, context_status_human_lines,
    format_model_budget_status, gateway_status_human_lines, mcp_status_human_lines,
    memory_status_human_lines, model_status_human_lines, print_api_status,
    print_api_status_for_human, print_approval_status, print_context_status,
    print_context_status_for_human, print_diff_status, print_diff_status_for_human,
    print_gateway_status, print_gateway_status_for_human, print_mcp_status,
    print_mcp_status_for_human, print_memory_status, print_memory_status_for_human,
    print_model_status, print_model_status_for_human, print_provider_status_for_human,
    print_rag_status, print_rag_status_for_human, print_replay_status,
    print_replay_status_for_human, print_screen_status_with_state, print_session_export,
    print_session_history, print_session_status, print_session_summaries, print_tasks_status,
    print_tools_status, print_tools_status_for_human, print_trace_status,
    print_trace_status_for_human, print_workbench_status, print_workbench_status_for_human,
    provider_status_human_lines, rag_status_human_lines, selected_screen_primary_action,
    session_history_human_lines, session_status_human_lines, session_summaries_human_lines,
    tasks_status_human_lines, tools_status_human_lines, workbench_status_human_lines,
};

pub(super) const MULTILINE_TERMINATOR: &str = ".";

pub(super) fn workbench_history_path(paths: &IkarosPaths) -> PathBuf {
    paths.home.join("workbench").join("history.txt")
}

pub(super) fn append_workbench_history(paths: &IkarosPaths, input: &str) -> Result<PathBuf> {
    let path = workbench_history_path(paths);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
    let input = redact_secrets(input.trim());
    writeln!(file, "{input}")?;
    writeln!(file, "---")?;
    Ok(path)
}

pub(super) fn load_workbench_history_entries(
    paths: &IkarosPaths,
    limit: usize,
) -> Result<Vec<String>> {
    let path = workbench_history_path(paths);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)?;
    let mut entries = content
        .split("\n---\n")
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(terminal_message)
        .collect::<Vec<_>>();
    let limit = limit.max(1);
    let start = entries.len().saturating_sub(limit);
    Ok(entries.split_off(start))
}

pub(super) fn print_workbench_input_history(paths: &IkarosPaths, limit: usize) -> Result<()> {
    let path = workbench_history_path(paths);
    let entries = load_workbench_history_entries(paths, limit)?;
    if entries.is_empty() {
        println!("workbench_input_history: 0");
        println!("workbench_history: {}", path_display(&path));
        return Ok(());
    }
    println!("workbench_input_history: {}", entries.len());
    println!("workbench_history: {}", path_display(&path));
    for (index, entry) in entries.iter().enumerate() {
        println!("- index={} input={}", index + 1, entry);
    }
    Ok(())
}

pub(super) fn workbench_input_history_human_lines(
    paths: &IkarosPaths,
    limit: usize,
) -> Result<Vec<String>> {
    let entries = load_workbench_history_entries(paths, limit)?;
    let mut lines = vec!["• History".to_owned()];
    if entries.is_empty() {
        lines.push("  no previous input".to_owned());
        return Ok(lines);
    }
    for (index, entry) in entries.iter().enumerate() {
        lines.push(format!("  {}. {}", index + 1, terminal_inline(entry)));
    }
    Ok(lines)
}

pub(super) fn format_workbench_help() -> String {
    format_slash_command_help()
}

pub(super) fn normalize_session_id(input: &str) -> String {
    redact_secrets(input)
        .trim()
        .replace(['/', '\\', ':', '\n', '\r', '\t'], "_")
}

pub(super) fn path_display(path: &Path) -> String {
    terminal_inline(&path.display().to_string())
}

pub(super) fn session_entry_cell(entry: &SessionEntry) -> WorkbenchCell {
    WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: format!(
            "entry {:?} turn={}",
            entry.kind,
            optional_turn(&entry.turn_id)
        ),
        detail: entry
            .visible_text
            .as_deref()
            .map(terminal_inline)
            .unwrap_or_else(|| "none".into()),
    }
}

pub(super) fn agent_event_cell(event: &AgentEvent) -> WorkbenchCell {
    let correlation_id = agent_event_correlation_id(event);
    let (title_extra, detail) = match &event.kind {
        AgentEventKind::ModelStream(stream_event) => {
            let label = format!("stream_event={}", model_stream_event_label(stream_event));
            let detail = model_stream_event_detail(stream_event, &correlation_id, event);
            (Some(label), detail)
        }
        AgentEventKind::ModelDiagnostic(diagnostic) => {
            let label = format!("diagnostic={}", terminal_inline(&diagnostic.kind));
            let detail = if diagnostic.message.trim().is_empty() {
                format!(
                    "{} correlation={} event={}",
                    label,
                    correlation_id,
                    terminal_inline(event.event_id.as_str())
                )
            } else {
                format!(
                    "{} correlation={} message={} event={}",
                    label,
                    correlation_id,
                    terminal_inline(&diagnostic.message),
                    terminal_inline(event.event_id.as_str())
                )
            };
            (Some(label), detail)
        }
        AgentEventKind::ToolCallStarted
        | AgentEventKind::ToolCallOutputDelta
        | AgentEventKind::ToolCallCompleted
        | AgentEventKind::ToolCallFailed
        | AgentEventKind::ToolCallCancelled => {
            let detail = tool_progress_detail(event, &correlation_id);
            let label = format!(
                "tool={} status={}",
                json_str(&event.payload, "name").unwrap_or("unknown"),
                json_str(&event.payload, "status")
                    .unwrap_or_else(|| default_tool_event_status(&event.kind))
            );
            (Some(terminal_inline(&label)), detail)
        }
        AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted => {
            let detail = context_progress_detail(event, &correlation_id);
            let label = match event.kind {
                AgentEventKind::ContextDiff => format!(
                    "sections={} references={}",
                    json_array_len(&event.payload, "sections"),
                    json_array_len(&event.payload, "references")
                ),
                AgentEventKind::ContextCompacted => format!(
                    "compressed_sections={}",
                    json_array_len(&event.payload, "compressed_sections")
                ),
                _ => unreachable!("context progress branch only receives context events"),
            };
            (Some(terminal_inline(&label)), detail)
        }
        AgentEventKind::ContinuationStarted
        | AgentEventKind::ContinuationCompleted
        | AgentEventKind::ContinuationFailed
        | AgentEventKind::ContinuationCancelled => {
            let detail = continuation_progress_detail(event, &correlation_id);
            let label = format!(
                "continuation_id={} status={}",
                json_str(&event.payload, "continuation_id").unwrap_or("unknown"),
                json_str(&event.payload, "status")
                    .unwrap_or_else(|| { default_continuation_event_status(&event.kind) })
            );
            (Some(terminal_inline(&label)), detail)
        }
        AgentEventKind::Error => {
            let detail = error_event_detail(event, &correlation_id);
            let label = format!(
                "phase={}",
                terminal_inline(json_str(&event.payload, "phase").unwrap_or("unknown"))
            );
            (Some(label), detail)
        }
        _ => (
            None,
            format!(
                "correlation={} event={}",
                correlation_id,
                terminal_inline(event.event_id.as_str())
            ),
        ),
    };
    let title = if let Some(extra) = title_extra {
        format!(
            "event {} {} turn={}",
            agent_event_label(&event.kind),
            extra,
            terminal_inline(event.turn_id.as_str())
        )
    } else {
        format!(
            "event {} turn={}",
            agent_event_label(&event.kind),
            terminal_inline(event.turn_id.as_str())
        )
    };
    WorkbenchCell {
        kind: agent_event_cell_kind(&event.kind),
        title,
        detail,
    }
}

fn error_event_detail(event: &AgentEvent, correlation_id: &str) -> String {
    let phase = json_str(&event.payload, "phase").unwrap_or("unknown");
    let message = json_str(&event.payload, "message").unwrap_or("unknown error");
    let error_kind = workbench_error_kind(message);
    format!(
        "phase={} kind={} message={} {} correlation={} event={}",
        terminal_inline(phase),
        error_kind,
        render_terminal_markdown(message),
        error_recovery_commands(error_kind, message),
        correlation_id,
        terminal_inline(event.event_id.as_str())
    )
}

fn workbench_error_kind(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("cancelled") || lower.contains("canceled") {
        "cancelled"
    } else {
        super::interactive_chat_turn_error_kind(message)
    }
}

fn error_recovery_commands(error_kind: &str, message: &str) -> String {
    match error_kind {
        "budget_exceeded" => {
            let raise = super::suggested_budget_command(message)
                .unwrap_or_else(|| "/budget set <tokens>".into());
            format!(
                "command=/status budget=/budget raise={} disable=/budget disable trace=/trace --failed",
                terminal_inline(&raise)
            )
        }
        "provider_error" => {
            "command=/provider debug health=/provider health --live trace=/trace --failed".into()
        }
        "unsupported_content" => {
            "command=/attach list clear=/attach clear matrix=/provider matrix trace=/trace --failed"
                .into()
        }
        "cancelled" => "command=/trace --failed".into(),
        _ => "command=/trace --failed".into(),
    }
}

fn model_stream_event_label(event: &ModelStreamEvent) -> &'static str {
    match event {
        ModelStreamEvent::Start { .. } => "start",
        ModelStreamEvent::TextDelta(_) => "text_delta",
        ModelStreamEvent::ReasoningDelta(_) => "reasoning_delta",
        ModelStreamEvent::ToolCallStart { .. } => "tool_call_start",
        ModelStreamEvent::ToolCallDelta { .. } => "tool_call_delta",
        ModelStreamEvent::ToolCallEnd { .. } => "tool_call_end",
        ModelStreamEvent::RefusalDelta(_) => "refusal_delta",
        ModelStreamEvent::Usage(_) => "usage",
        ModelStreamEvent::Error { .. } => "error",
        ModelStreamEvent::Done => "done",
    }
}

fn model_stream_event_detail(
    stream_event: &ModelStreamEvent,
    correlation_id: &str,
    event: &AgentEvent,
) -> String {
    let event_id = terminal_inline(event.event_id.as_str());
    match stream_event {
        ModelStreamEvent::Start { provider, model } => format!(
            "provider={} model={} correlation={} event={}",
            terminal_inline(provider),
            terminal_inline(model),
            correlation_id,
            event_id
        ),
        ModelStreamEvent::TextDelta(text) => format!(
            "markdown={} correlation={} event={}",
            render_terminal_markdown(text),
            correlation_id,
            event_id
        ),
        ModelStreamEvent::ReasoningDelta(text) => format!(
            "reasoning_markdown={} correlation={} event={}",
            render_terminal_markdown(text),
            correlation_id,
            event_id
        ),
        ModelStreamEvent::RefusalDelta(text) => format!(
            "refusal_markdown={} correlation={} event={}",
            render_terminal_markdown(text),
            correlation_id,
            event_id
        ),
        ModelStreamEvent::ToolCallStart { id, name } => format!(
            "tool_call_id={} name={} correlation={} event={}",
            terminal_inline(id),
            terminal_inline(name),
            correlation_id,
            event_id
        ),
        ModelStreamEvent::ToolCallDelta { id, args_delta } => format!(
            "tool_call_id={} args_delta={} correlation={} event={}",
            terminal_inline(id),
            terminal_inline(args_delta),
            correlation_id,
            event_id
        ),
        ModelStreamEvent::ToolCallEnd { id } => format!(
            "tool_call_id={} correlation={} event={}",
            terminal_inline(id),
            correlation_id,
            event_id
        ),
        ModelStreamEvent::Usage(usage) => format!(
            "prompt_tokens={} completion_tokens={} total_tokens={} cache_read_tokens={} cache_write_tokens={} correlation={} event={}",
            usage
                .prompt_tokens
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "unknown".into()),
            usage
                .completion_tokens
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "unknown".into()),
            usage
                .total_tokens
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "unknown".into()),
            usage
                .cache_read_tokens
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "unknown".into()),
            usage
                .cache_write_tokens
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "unknown".into()),
            correlation_id,
            event_id
        ),
        ModelStreamEvent::Error { message } => format!(
            "error_markdown={} correlation={} event={}",
            render_terminal_markdown(message),
            correlation_id,
            event_id
        ),
        ModelStreamEvent::Done => format!("correlation={} event={}", correlation_id, event_id),
    }
}

fn tool_progress_detail(event: &AgentEvent, correlation_id: &str) -> String {
    let payload = &event.payload;
    let tool = json_str(payload, "name").unwrap_or("unknown");
    let status =
        json_str(payload, "status").unwrap_or_else(|| default_tool_event_status(&event.kind));
    let call = json_str(payload, "tool_call_id")
        .or_else(|| json_str(payload, "id"))
        .unwrap_or("unknown");
    let mode = json_str(payload, "execution_mode").unwrap_or("unknown");
    let timeout = json_u64(payload, "timeout_ms")
        .map(|timeout| timeout.to_string())
        .unwrap_or_else(|| "none".into());
    let summary = json_str(payload, "summary").unwrap_or("none");
    format!(
        "tool={} status={} call={} mode={} timeout_ms={} summary={} correlation={} event={}",
        terminal_inline(tool),
        terminal_inline(status),
        terminal_inline(call),
        terminal_inline(mode),
        timeout,
        terminal_inline(summary),
        correlation_id,
        terminal_inline(event.event_id.as_str())
    )
}

fn default_tool_event_status(kind: &AgentEventKind) -> &'static str {
    match kind {
        AgentEventKind::ToolCallStarted => "started",
        AgentEventKind::ToolCallOutputDelta => "output",
        AgentEventKind::ToolCallCompleted => "completed",
        AgentEventKind::ToolCallFailed => "failed",
        AgentEventKind::ToolCallCancelled => "cancelled",
        _ => "unknown",
    }
}

fn continuation_progress_detail(event: &AgentEvent, correlation_id: &str) -> String {
    let payload = &event.payload;
    let continuation_id = json_str(payload, "continuation_id").unwrap_or("unknown");
    let kind = json_str(payload, "kind").unwrap_or("unknown");
    let status = json_str(payload, "status")
        .unwrap_or_else(|| default_continuation_event_status(&event.kind));
    let reason = json_str(payload, "reason").unwrap_or("none");
    let attempts = json_u64(payload, "attempt_count")
        .map(|attempts| attempts.to_string())
        .unwrap_or_else(|| "unknown".into());
    format!(
        "continuation_id={} continuation_kind={} status={} reason={} attempts={} correlation={} event={}",
        terminal_inline(continuation_id),
        terminal_inline(kind),
        terminal_inline(status),
        terminal_inline(reason),
        attempts,
        correlation_id,
        terminal_inline(event.event_id.as_str())
    )
}

fn default_continuation_event_status(kind: &AgentEventKind) -> &'static str {
    match kind {
        AgentEventKind::ContinuationStarted => "started",
        AgentEventKind::ContinuationCompleted => "completed",
        AgentEventKind::ContinuationFailed => "failed",
        AgentEventKind::ContinuationCancelled => "cancelled",
        _ => "unknown",
    }
}

fn context_progress_detail(event: &AgentEvent, correlation_id: &str) -> String {
    match event.kind {
        AgentEventKind::ContextDiff => {
            let budget = event
                .payload
                .get("budget")
                .unwrap_or(&serde_json::Value::Null);
            format!(
                "sections={} references={} used={} max={} context_window={} estimator={} compressed={} continuation_prompt={} correlation={} event={}",
                json_array_len(&event.payload, "sections"),
                json_array_len(&event.payload, "references"),
                json_u64(budget, "used_tokens").unwrap_or(0),
                json_u64(budget, "max_tokens").unwrap_or(0),
                json_u64(budget, "context_window").unwrap_or(0),
                terminal_inline(json_str(budget, "estimator").unwrap_or("unknown")),
                yes_no(
                    event.payload.get("compression_summary").is_some()
                        || json_array_len(&event.payload, "compressed_sections") > 0
                ),
                yes_no(event.payload.get("continuation_prompt").is_some()),
                correlation_id,
                terminal_inline(event.event_id.as_str())
            )
        }
        AgentEventKind::ContextCompacted => format!(
            "compressed_sections={} continuation_prompt={} summary={} correlation={} event={}",
            json_array_len(&event.payload, "compressed_sections"),
            yes_no(event.payload.get("continuation_prompt").is_some()),
            terminal_inline(json_str(&event.payload, "summary").unwrap_or("none")),
            correlation_id,
            terminal_inline(event.event_id.as_str())
        ),
        _ => format!(
            "correlation={} event={}",
            correlation_id,
            terminal_inline(event.event_id.as_str())
        ),
    }
}

fn json_str<'a>(payload: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(serde_json::Value::as_str)
}

fn json_u64(payload: &serde_json::Value, key: &str) -> Option<u64> {
    payload.get(key).and_then(serde_json::Value::as_u64)
}

fn json_array_len(payload: &serde_json::Value, key: &str) -> usize {
    payload
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn agent_event_correlation_id(event: &AgentEvent) -> String {
    format!(
        "session:{}:turn:{}",
        terminal_inline(event.session_id.as_str()),
        terminal_inline(event.turn_id.as_str())
    )
}

pub(super) fn coding_event_cells<'a>(
    events: impl IntoIterator<Item = &'a AgentEvent>,
) -> Vec<(&'static str, WorkbenchCell)> {
    events
        .into_iter()
        .filter(|event| matches!(event.kind, AgentEventKind::CodingTurn))
        .filter_map(|event| {
            let kind = event
                .payload
                .get("kind")
                .and_then(serde_json::Value::as_str)?;
            let group = coding_event_group(kind);
            let summary = event
                .payload
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(kind);
            Some((
                group,
                WorkbenchCell {
                    kind: WorkbenchCellKind::Coding,
                    title: format!("coding {group}"),
                    detail: format!(
                        "turn={} correlation={} kind={} summary={}",
                        terminal_inline(event.turn_id.as_str()),
                        agent_event_correlation_id(event),
                        terminal_inline(kind),
                        terminal_inline(summary)
                    ),
                },
            ))
        })
        .collect()
}

fn coding_event_group(kind: &str) -> &'static str {
    match kind {
        "patch_applied" | "patch_failed" | "patch_skipped" | "diff_updated" => "diff",
        "test_evidence_recorded" => "test",
        "review_started" | "review_finding" | "review_completed" | "iteration_planned" => "review",
        _ => "progress",
    }
}

fn optional_turn(turn_id: &Option<ikaros_session::TurnId>) -> String {
    turn_id
        .as_ref()
        .map(|turn_id| terminal_inline(turn_id.as_str()))
        .unwrap_or_else(|| "none".into())
}

fn agent_event_cell_kind(kind: &AgentEventKind) -> WorkbenchCellKind {
    match kind {
        AgentEventKind::ModelStream(_) | AgentEventKind::ModelDiagnostic(_) => {
            WorkbenchCellKind::Model
        }
        AgentEventKind::ToolCallStarted
        | AgentEventKind::ToolCallOutputDelta
        | AgentEventKind::ToolCallCompleted
        | AgentEventKind::ToolCallFailed
        | AgentEventKind::ToolCallCancelled => WorkbenchCellKind::Tool,
        AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted => {
            WorkbenchCellKind::Context
        }
        AgentEventKind::MemoryLifecycle => WorkbenchCellKind::Memory,
        AgentEventKind::CodingTurn => WorkbenchCellKind::Coding,
        AgentEventKind::AuditAnchor => WorkbenchCellKind::Audit,
        AgentEventKind::ContinuationStarted
        | AgentEventKind::ContinuationCompleted
        | AgentEventKind::ContinuationFailed
        | AgentEventKind::ContinuationCancelled => WorkbenchCellKind::Continuation,
        AgentEventKind::ApprovalRequested | AgentEventKind::ApprovalResolved => {
            WorkbenchCellKind::Approval
        }
        AgentEventKind::Error => WorkbenchCellKind::Error,
        AgentEventKind::SessionStart
        | AgentEventKind::TurnStart
        | AgentEventKind::UserMessage
        | AgentEventKind::TurnEnd => WorkbenchCellKind::Session,
    }
}

fn agent_event_label(kind: &AgentEventKind) -> &'static str {
    match kind {
        AgentEventKind::SessionStart => "session_start",
        AgentEventKind::TurnStart => "turn_start",
        AgentEventKind::UserMessage => "user_message",
        AgentEventKind::ModelStream(_) => "model_stream",
        AgentEventKind::ModelDiagnostic(_) => "model_diagnostic",
        AgentEventKind::ToolCallStarted => "tool_call_started",
        AgentEventKind::ToolCallOutputDelta => "tool_call_output_delta",
        AgentEventKind::ToolCallCompleted => "tool_call_completed",
        AgentEventKind::ToolCallFailed => "tool_call_failed",
        AgentEventKind::ToolCallCancelled => "tool_call_cancelled",
        AgentEventKind::ContextDiff => "context_diff",
        AgentEventKind::ContextCompacted => "context_compacted",
        AgentEventKind::MemoryLifecycle => "memory_lifecycle",
        AgentEventKind::CodingTurn => "coding_turn",
        AgentEventKind::AuditAnchor => "audit_anchor",
        AgentEventKind::ContinuationStarted => "continuation_started",
        AgentEventKind::ContinuationCompleted => "continuation_completed",
        AgentEventKind::ContinuationFailed => "continuation_failed",
        AgentEventKind::ContinuationCancelled => "continuation_cancelled",
        AgentEventKind::ApprovalRequested => "approval_requested",
        AgentEventKind::ApprovalResolved => "approval_resolved",
        AgentEventKind::TurnEnd => "turn_end",
        AgentEventKind::Error => "error",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_session::{EventId, SessionEntryKind, SessionId, TurnId};
    use tempfile::tempdir;

    #[test]
    fn workbench_history_redacts_secret_like_input() {
        let temp = tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path());

        let path = append_workbench_history(&paths, "token sk-secret-value").expect("append");

        let raw = fs::read_to_string(path).expect("history");
        assert!(!raw.contains("sk-secret-value"));
        assert!(raw.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn workbench_history_preserves_multiline_entries_without_secret_leakage() {
        let temp = tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path());

        append_workbench_history(&paths, "first\napi_key=sk-secret-value\nthird").expect("append");

        let entries = load_workbench_history_entries(&paths, 10).expect("history entries");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0], "first\n[REDACTED_SECRET]\nthird");
    }

    #[test]
    fn terminal_inline_redacts_and_replaces_control_characters() {
        let rendered = terminal_inline("token sk-secret-value\nnext\tcell\r");

        assert!(!rendered.contains("sk-secret-value"));
        assert!(!rendered.chars().any(char::is_control));
        assert_eq!(rendered, "token [REDACTED_SECRET]_next_cell_");
    }

    #[test]
    fn terminal_message_strips_mouse_escape_sequences() {
        let rendered = terminal_message("\u{1b}[<35;55;37Mhello\u{1b}[<35;55;37m");
        assert_eq!(rendered, "hello");
    }

    #[test]
    fn terminal_message_strips_bare_mouse_escape_fragments() {
        let rendered = terminal_message("[<35;55;37Mhello[<35;55;37m");
        assert_eq!(rendered, "hello");
    }

    #[test]
    fn terminal_message_strips_double_open_mouse_escape_fragments() {
        let rendered = terminal_message("[[<35;55;37Mhello[[<35;55;37m");
        assert_eq!(rendered, "hello");
    }

    #[test]
    fn terminal_message_drops_partial_mouse_escape_tail() {
        let rendered = terminal_message("hello[<35;55");
        assert_eq!(rendered, "hello");
    }

    #[test]
    fn session_ids_are_normalized_for_terminal_resume() {
        assert_eq!(
            normalize_session_id("gateway/thread:one\n"),
            "gateway_thread_one"
        );
    }

    #[test]
    fn session_entry_cells_redact_visible_text() {
        let mut entry = SessionEntry::new(
            SessionId::from("session-one"),
            SessionEntryKind::UserMessage,
        );
        entry.turn_id = Some(TurnId::from("turn-one"));
        entry.visible_text = Some("use key sk-secret-value".into());

        let rendered = session_entry_cell(&entry).render();

        assert!(rendered.contains("kind=session"));
        assert!(rendered.contains("turn=turn-one"));
        assert!(!rendered.contains("sk-secret-value"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn agent_event_cells_group_coding_events() {
        let event = AgentEvent {
            event_id: EventId::from("event-one"),
            session_id: SessionId::from("session-one"),
            turn_id: TurnId::from("turn-one"),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Runtime,
            kind: AgentEventKind::CodingTurn,
            payload: serde_json::Value::Null,
        };

        let rendered = agent_event_cell(&event).render();

        assert!(rendered.contains("kind=coding"));
        assert!(rendered.contains("coding_turn"));
        assert!(rendered.contains("turn=turn-one"));
    }

    #[test]
    fn agent_event_cell_renders_tool_progress_payload_without_secret_leakage() {
        let event = AgentEvent {
            event_id: EventId::from("event-tool"),
            session_id: SessionId::from("session-tool"),
            turn_id: TurnId::from("turn-tool"),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Tool,
            kind: AgentEventKind::ToolCallFailed,
            payload: serde_json::json!({
                "tool_call_id": "call-123",
                "tool_event_id": "event-started",
                "name": "shell_exec",
                "status": "failed",
                "ok": false,
                "execution_mode": "sequential",
                "timeout_ms": 5000,
                "summary": "command failed with api_key=sk-secret-value",
                "output": {
                    "error": "exit 1 token=sk-secret-value",
                    "recoverable": true
                }
            }),
        };

        let rendered = agent_event_cell(&event).render();

        assert!(rendered.contains("kind=tool"));
        assert!(rendered.contains("tool_call_failed"));
        assert!(rendered.contains("tool=shell_exec"));
        assert!(rendered.contains("status=failed"));
        assert!(rendered.contains("call=call-123"));
        assert!(rendered.contains("mode=sequential"));
        assert!(rendered.contains("timeout_ms=5000"));
        assert!(rendered.contains("summary=command failed"));
        assert!(!rendered.contains("sk-secret-value"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn agent_event_cell_renders_continuation_payload_without_secret_leakage() {
        let event = AgentEvent {
            event_id: EventId::from("event-continuation"),
            session_id: SessionId::from("session-continuation"),
            turn_id: TurnId::from("turn-continuation"),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Runtime,
            kind: AgentEventKind::ContinuationCancelled,
            payload: serde_json::json!({
                "continuation_id": "continuation-one",
                "kind": "next_turn",
                "status": "cancelled",
                "reason": "operator cancelled token=sk-secret-value",
                "attempt_count": 2,
            }),
        };

        let rendered = agent_event_cell(&event).render();

        assert!(rendered.contains("kind=continuation"));
        assert!(rendered.contains("continuation_cancelled"));
        assert!(rendered.contains("continuation_id=continuation-one"));
        assert!(rendered.contains("continuation_kind=next_turn"));
        assert!(rendered.contains("status=cancelled"));
        assert!(rendered.contains("reason=operator cancelled"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(rendered.contains("attempts=2"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn agent_event_cell_renders_context_progress_payload_without_secret_leakage() {
        let event = AgentEvent {
            event_id: EventId::from("event-context"),
            session_id: SessionId::from("session-context"),
            turn_id: TurnId::from("turn-context"),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Context,
            kind: AgentEventKind::ContextDiff,
            payload: serde_json::json!({
                "budget": {
                    "used_tokens": 1024,
                    "max_tokens": 4096,
                    "context_window": 8192,
                    "estimator": "heuristic-v1"
                },
                "sections": [
                    {"kind": "history"},
                    {"kind": "references", "lines": ["token sk-secret-value"]}
                ],
                "references": [
                    {"raw": "@file:src/lib.rs:1-4"}
                ],
                "compression_summary": "compressed token sk-secret-value",
                "continuation_prompt": "continue"
            }),
        };

        let rendered = agent_event_cell(&event).render();

        assert!(rendered.contains("kind=context"));
        assert!(rendered.contains("context_diff"));
        assert!(rendered.contains("sections=2"));
        assert!(rendered.contains("references=1"));
        assert!(rendered.contains("used=1024"));
        assert!(rendered.contains("max=4096"));
        assert!(rendered.contains("context_window=8192"));
        assert!(rendered.contains("estimator=heuristic-v1"));
        assert!(rendered.contains("compressed=yes"));
        assert!(rendered.contains("continuation_prompt=yes"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn agent_event_cell_renders_model_stream_markdown_without_secret_leakage() {
        let event = AgentEvent {
            event_id: EventId::from("event-model"),
            session_id: SessionId::from("session-model"),
            turn_id: TurnId::from("turn-model"),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Model,
            kind: AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::TextDelta(
                "Here is code:\n\n```rust\nlet token = \"sk-secret-value\";\n```\n\n| File | Status |\n| --- | --- |\n| src/lib.rs | changed |\n".into(),
            )),
            payload: serde_json::Value::Null,
        };

        let rendered = agent_event_cell(&event).render();

        assert!(rendered.contains("kind=model"));
        assert!(rendered.contains("model_stream"));
        assert!(rendered.contains("stream_event=text_delta"));
        assert!(rendered.contains("╭─ rust"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(rendered.contains("File"));
        assert!(rendered.contains("Status"));
        assert!(rendered.contains("src/lib.rs"));
        assert!(rendered.contains("changed"));
        assert!(!rendered.contains("[code"));
        assert!(!rendered.contains("[table]"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn agent_event_cell_renders_error_phase_message_and_recovery_actions() {
        let event = AgentEvent {
            event_id: EventId::from("event-error"),
            session_id: SessionId::from("session-error"),
            turn_id: TurnId::from("turn-error"),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Runtime,
            kind: AgentEventKind::Error,
            payload: serde_json::json!({
                "phase": "model_call",
                "message": "temporary failure in name resolution api_key=sk-secret-value"
            }),
        };

        let rendered = agent_event_cell(&event).render();

        assert!(rendered.contains("kind=error"));
        assert!(rendered.contains("error"));
        assert!(rendered.contains("phase=model_call"));
        assert!(rendered.contains("kind=provider_error"));
        assert!(rendered.contains("/provider debug"));
        assert!(rendered.contains("/provider health --live"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn coding_event_cells_group_diff_test_review_and_progress() {
        let events = [
            "diff_updated",
            "test_evidence_recorded",
            "review_completed",
            "plan_prepared",
        ]
        .into_iter()
        .map(|kind| AgentEvent {
            event_id: EventId::new(),
            session_id: SessionId::from("session-one"),
            turn_id: TurnId::from("turn-one"),
            parent_event_id: None,
            at: time::OffsetDateTime::now_utc(),
            source: ikaros_session::AgentEventSource::Tool,
            kind: AgentEventKind::CodingTurn,
            payload: serde_json::json!({
                "kind": kind,
                "summary": format!("summary for {kind}"),
            }),
        })
        .collect::<Vec<_>>();

        let cells = coding_event_cells(&events);
        let groups = cells.iter().map(|(group, _)| *group).collect::<Vec<_>>();

        assert_eq!(groups, vec!["diff", "test", "review", "progress"]);
        assert!(
            cells
                .iter()
                .any(|(_, cell)| cell.render().contains("title=coding diff"))
        );
    }

    #[test]
    fn workbench_snapshot_wraps_cells_and_redacts_secret_text() {
        let snapshot = render_workbench_snapshot(
            &[
                WorkbenchCell {
                    kind: WorkbenchCellKind::Coding,
                    title: "coding diff".into(),
                    detail: "turn=turn-one kind=diff_updated summary=changed src/lib.rs with sk-secret-value".into(),
                },
                WorkbenchCell {
                    kind: WorkbenchCellKind::Approval,
                    title: "approval pending".into(),
                    detail: "provider_call=true workspace_write=true shell=false".into(),
                },
            ],
            42,
        );

        assert_eq!(
            snapshot,
            "snapshot width=42\n[coding] coding diff\n  turn=turn-one kind=diff_updated\n  summary=changed src/lib.rs with\n  [REDACTED_SECRET]\n[approval] approval pending\n  provider_call=true workspace_write=true\n  shell=false\n"
        );
    }

    #[test]
    fn workbench_snapshot_splits_long_tokens_at_width() {
        let snapshot = render_workbench_snapshot(
            &[WorkbenchCell {
                kind: WorkbenchCellKind::Context,
                title: "context reference".into(),
                detail: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            }],
            18,
        );

        for line in snapshot.lines().skip(1) {
            assert!(
                line.chars().count() <= 18,
                "snapshot line exceeds width: {line}"
            );
        }
    }

    #[test]
    fn fullscreen_workbench_frame_renders_core_panels_without_secret_leakage() {
        let screen = WorkbenchScreen {
            title: "Ikaros Workbench".into(),
            status: vec![
                WorkbenchCell {
                    kind: WorkbenchCellKind::Model,
                    title: "model".into(),
                    detail: "provider=openai-compatible model=kimi-k2.6".into(),
                },
                WorkbenchCell {
                    kind: WorkbenchCellKind::Continuation,
                    title: "queue".into(),
                    detail: "running=1 pending=2".into(),
                },
            ],
            timeline: vec![WorkbenchCell {
                kind: WorkbenchCellKind::Coding,
                title: "coding test failed".into(),
                detail: "turn=turn-one cargo test failed".into(),
            }],
            main: vec![WorkbenchCell {
                kind: WorkbenchCellKind::Context,
                title: "context".into(),
                detail: "budget=8192 references=2".into(),
            }],
            side: vec![WorkbenchCell {
                kind: WorkbenchCellKind::Approval,
                title: "approval pending".into(),
                detail: "write=true shell=true token=sk-secret-value".into(),
            }],
            footer: "session=session-one /approval approve <id>".into(),
            input_hint: "/code apply --run-tests".into(),
        };

        let frame = render_fullscreen_workbench_with_state(
            &screen,
            &WorkbenchScreenState::default(),
            72,
            18,
        );

        assert!(frame.contains("Ikaros"));
        assert!(frame.contains("kimi-k2.6"));
        assert!(frame.contains("Ask Ikaros to do anything"));
        assert!(!frame.contains("Timeline"));
        assert!(!frame.contains("Approvals / Queue"));
        assert!(!frame.contains("selected panel="));
        assert!(!frame.contains("actions selector="));
        assert!(!frame.contains("sk-secret-value"));
        for line in frame.lines() {
            assert!(
                line.chars().count() <= 72,
                "frame line exceeds width: {line}"
            );
        }
        assert_eq!(frame.lines().count(), 18);
    }

    #[test]
    fn workbench_input_state_recalls_history_and_completes_slash_commands() {
        let mut input = WorkbenchInputState::default();
        input.record_history("first message");
        input.record_history("/status");

        assert_eq!(
            input.apply(WorkbenchInputAction::HistoryPrevious),
            Some("/status".into())
        );
        assert_eq!(
            input.apply(WorkbenchInputAction::HistoryPrevious),
            Some("first message".into())
        );
        assert_eq!(
            input.apply(WorkbenchInputAction::HistoryNext),
            Some("/status".into())
        );

        input.set_buffer("/sta");
        assert_eq!(input.completion_candidates(), vec!["/status"]);
        assert_eq!(
            input.apply(WorkbenchInputAction::Complete),
            Some("/status ".into())
        );
        assert_eq!(input.buffer(), "/status ");

        input.record_history("token=sk-secret-value");
        assert!(
            !input
                .history_entries()
                .iter()
                .any(|entry| entry.contains("sk-secret-value"))
        );
    }

    #[test]
    fn workbench_input_state_edits_at_cursor_and_undoes_last_change() {
        let mut input = WorkbenchInputState::default();

        input.insert_text("helo");
        assert_eq!(input.buffer(), "helo");
        assert_eq!(input.cursor(), 4);

        input.apply(WorkbenchInputAction::MoveLeft);
        input.insert_text("l");
        assert_eq!(input.buffer(), "hello");
        assert_eq!(input.cursor(), 4);

        input.apply(WorkbenchInputAction::DeletePrevious);
        assert_eq!(input.buffer(), "helo");
        assert_eq!(input.cursor(), 3);

        input.apply(WorkbenchInputAction::Undo);
        assert_eq!(input.buffer(), "hello");
        assert_eq!(input.cursor(), 4);

        input.apply(WorkbenchInputAction::MoveRight);
        input.apply(WorkbenchInputAction::DeleteNext);
        assert_eq!(input.buffer(), "hello");
        assert_eq!(input.cursor(), 5);
    }

    #[test]
    fn workbench_input_state_completion_keeps_cursor_at_end() {
        let mut input = WorkbenchInputState::default();

        input.insert_text("/sta");
        assert_eq!(
            input.apply(WorkbenchInputAction::Complete),
            Some("/status ".into())
        );

        assert_eq!(input.buffer(), "/status ");
        assert_eq!(input.cursor(), "/status ".chars().count());
        input.apply(WorkbenchInputAction::Undo);
        assert_eq!(input.buffer(), "/sta");
        assert_eq!(input.cursor(), 4);
    }

    #[test]
    fn workbench_input_state_renders_cursor_view_and_completion_candidates() {
        let mut input = WorkbenchInputState::default();
        input.insert_text("/sta");
        input.apply(WorkbenchInputAction::MoveLeft);

        let line = format_workbench_input_state("move_left", &input);

        assert!(line.contains("input_state: action=move_left"));
        assert!(line.contains("cursor=3"));
        assert!(line.contains("buffer=/sta"));
        assert!(line.contains("view=/st|a"));
        assert!(line.contains("completion_candidates=/status"));

        input.set_buffer("token=sk-secret-value");
        let redacted = format_workbench_input_state("set_buffer", &input);
        assert!(redacted.contains("[REDACTED_SECRET]"));
        assert!(!redacted.contains("sk-secret-value"));
    }

    #[test]
    fn workbench_input_state_moves_to_line_start_and_end() {
        let mut input = WorkbenchInputState::default();

        input.insert_text("abc");
        input.apply(WorkbenchInputAction::MoveStart);
        input.insert_text(">");
        assert_eq!(input.buffer(), ">abc");
        assert_eq!(input.cursor(), 1);

        input.apply(WorkbenchInputAction::MoveEnd);
        input.insert_text("<");
        assert_eq!(input.buffer(), ">abc<");
        assert_eq!(input.cursor(), 5);
    }

    #[test]
    fn workbench_input_state_kills_text_around_cursor() {
        let mut input = WorkbenchInputState::default();

        input.insert_text("hello world");
        for _ in 0..5 {
            input.apply(WorkbenchInputAction::MoveLeft);
        }
        input.apply(WorkbenchInputAction::DeleteBeforeCursor);
        assert_eq!(input.buffer(), "world");
        assert_eq!(input.cursor(), 0);

        input.apply(WorkbenchInputAction::Undo);
        assert_eq!(input.buffer(), "hello world");
        assert_eq!(input.cursor(), 6);

        input.apply(WorkbenchInputAction::MoveLeft);
        input.apply(WorkbenchInputAction::DeleteAfterCursor);
        assert_eq!(input.buffer(), "hello");
        assert_eq!(input.cursor(), 5);
    }

    #[test]
    fn workbench_input_event_adapter_maps_terminal_control_sequences() {
        assert_eq!(
            parse_workbench_input_event("\u{1b}[A"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::CompletionPrevious)
        );
        assert_eq!(
            parse_workbench_input_event("\u{1b}[B"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::CompletionNext)
        );
        assert_eq!(
            parse_workbench_input_event("\u{1b}[D"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::MoveLeft)
        );
        assert_eq!(
            parse_workbench_input_event("\u{1b}[C"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::MoveRight)
        );
        assert_eq!(
            parse_workbench_input_event("\u{1b}[H"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::MoveStart)
        );
        assert_eq!(
            parse_workbench_input_event("\u{1b}[F"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::MoveEnd)
        );
        assert_eq!(
            parse_workbench_input_event("\u{7f}"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::DeletePrevious)
        );
        assert_eq!(
            parse_workbench_input_event("\u{1b}[3~"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteNext)
        );
        assert_eq!(
            parse_workbench_input_event("\u{10}"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::HistoryPrevious)
        );
        assert_eq!(
            parse_workbench_input_event("\u{e}"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::HistoryNext)
        );
        assert_eq!(
            parse_workbench_input_event("\u{2}"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::MoveLeft)
        );
        assert_eq!(
            parse_workbench_input_event("\u{6}"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::MoveRight)
        );
        assert_eq!(
            parse_workbench_input_event("\u{4}"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteNext)
        );
        assert_eq!(
            parse_workbench_input_event("\u{15}"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteBeforeCursor)
        );
        assert_eq!(
            parse_workbench_input_event("\u{b}"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteAfterCursor)
        );
        assert_eq!(
            parse_workbench_input_event("\u{1a}"),
            WorkbenchInputEvent::Action(WorkbenchInputAction::Undo)
        );
    }

    #[test]
    fn workbench_input_event_adapter_extracts_completion_prefix_without_redaction_leakage() {
        assert_eq!(
            parse_workbench_input_event("/sta\t"),
            WorkbenchInputEvent::CompletePrefix("/sta".into())
        );
        assert_eq!(
            parse_workbench_input_event("hello"),
            WorkbenchInputEvent::SubmitLine("hello".into())
        );
        assert_eq!(
            parse_workbench_input_event("token=sk-secret\t"),
            WorkbenchInputEvent::CompletePrefix("[REDACTED_SECRET]".into())
        );
    }

    #[test]
    fn workbench_input_state_loads_persisted_history_for_navigation() {
        let temp = tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path());
        append_workbench_history(&paths, "first message").expect("append first");
        append_workbench_history(&paths, "second token=sk-secret-value").expect("append second");

        let entries = load_workbench_history_entries(&paths, 8).expect("history entries");
        let mut input = WorkbenchInputState::from_history(entries);

        let selected = input
            .apply(WorkbenchInputAction::HistoryPrevious)
            .expect("latest history entry");
        assert!(selected.contains("second"));
        assert!(!selected.contains("sk-secret-value"));
        assert!(selected.contains("[REDACTED_SECRET]"));
    }
}

#[test]
fn agent_event_cell_renders_model_diagnostic_kind_and_message() {
    let event = AgentEvent {
        event_id: ikaros_session::EventId::from("event-diag"),
        session_id: ikaros_session::SessionId::from("session-diag"),
        turn_id: ikaros_session::TurnId::from("turn-diag"),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_session::AgentEventSource::Model,
        kind: AgentEventKind::ModelDiagnostic(ikaros_models::ModelRequestDiagnostic {
            kind: "fallback_provider_selected".into(),
            message: "provider openai-compatible/qwen-2.5-72b selected after 1 fallback attempt(s)"
                .into(),
            parameter: None,
        }),
        payload: serde_json::Value::Null,
    };

    let rendered = agent_event_cell(&event).render();

    assert!(rendered.contains("kind=model"));
    assert!(rendered.contains("model_diagnostic"));
    assert!(rendered.contains("turn=turn-diag"));
    assert!(rendered.contains("diagnostic=fallback_provider_selected"));
    assert!(rendered.contains("qwen-2.5-72b"));
}
