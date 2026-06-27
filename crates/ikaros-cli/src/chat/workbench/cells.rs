// SPDX-License-Identifier: GPL-3.0-only

use ikaros_providers::model::ModelStreamEvent;
use ikaros_state::session::{AgentEvent, AgentEventKind, SessionEntry};
use ikaros_terminal::{
    WorkbenchCell, WorkbenchCellKind,
    render_terminal_markdown_for_current_width as render_terminal_markdown, terminal_inline,
};

pub(in crate::chat) fn session_entry_cell(entry: &SessionEntry) -> WorkbenchCell {
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

pub(in crate::chat) fn agent_event_cell(event: &AgentEvent) -> WorkbenchCell {
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
        super::super::interactive_chat_turn_error_kind(message)
    }
}

fn error_recovery_commands(error_kind: &str, message: &str) -> String {
    match error_kind {
        "budget_exceeded" => {
            let raise = super::super::suggested_budget_command(message)
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

pub(in crate::chat) fn coding_event_cells<'a>(
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

fn optional_turn(turn_id: &Option<ikaros_state::session::TurnId>) -> String {
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
