// SPDX-License-Identifier: GPL-3.0-only

use ikaros_state::session::{AgentEvent, AgentEventKind, SessionEntry, SessionEntryKind};
use std::collections::BTreeMap;

use super::super::super::terminal_inline;
use super::request::TimelineRequest;
pub(super) fn human_replay_label(label: &str) -> &'static str {
    match label {
        "replay" => "Replay",
        "debug" => "Debug Timeline",
        _ => "Timeline",
    }
}

pub(super) fn print_human_request_filters(label: &str, request: &TimelineRequest) {
    if let Some(turn_id) = request.turn_filter.as_deref() {
        println!("  {label} turn: {}", terminal_inline(turn_id));
    }
    if let Some(kind) = request.kind_filter.as_deref() {
        println!("  {label} kind: {}", terminal_inline(kind));
    }
    if let Some(point) = request.point_filter.as_deref() {
        println!("  {label} point: {}", terminal_inline(point));
    }
}

pub(super) fn print_human_recent_entries<'a>(
    entries: impl IntoIterator<Item = &'a SessionEntry>,
    limit: usize,
) {
    let entries = entries.into_iter().collect::<Vec<_>>();
    if entries.is_empty() {
        println!("  recent entries: none");
        return;
    }
    println!("  recent entries:");
    let start = entries.len().saturating_sub(limit.max(1));
    for entry in &entries[start..] {
        println!(
            "  * {}: {}",
            human_session_entry_role(entry),
            terminal_inline(&single_line_excerpt(
                entry.visible_text.as_deref().unwrap_or("none"),
                96,
            ))
        );
    }
}

pub(super) fn print_human_recent_events<'a>(
    events: impl IntoIterator<Item = &'a AgentEvent>,
    limit: usize,
) {
    let events = events.into_iter().collect::<Vec<_>>();
    if events.is_empty() {
        println!("  recent events: none");
        return;
    }
    println!("  recent events:");
    let start = events.len().saturating_sub(limit.max(1));
    for event in &events[start..] {
        println!(
            "  * {}: turn {}",
            human_agent_event_label(&event.kind),
            terminal_inline(event.turn_id.as_str())
        );
    }
}

fn human_session_entry_role(entry: &SessionEntry) -> &'static str {
    match entry.kind {
        SessionEntryKind::AssistantMessage => "assistant",
        SessionEntryKind::UserMessage => "user",
        _ => "entry",
    }
}

fn human_agent_event_label(kind: &AgentEventKind) -> &'static str {
    match kind {
        AgentEventKind::ModelStream(_) => "model stream",
        AgentEventKind::ModelDiagnostic(_) => "model diagnostic",
        AgentEventKind::ToolCallStarted => "tool started",
        AgentEventKind::ToolCallOutputDelta => "tool output",
        AgentEventKind::ToolCallCompleted => "tool completed",
        AgentEventKind::ToolCallFailed => "tool failed",
        AgentEventKind::ToolCallCancelled => "tool cancelled",
        AgentEventKind::ContextDiff => "context updated",
        AgentEventKind::ContextCompacted => "context compacted",
        AgentEventKind::MemoryLifecycle => "memory lifecycle",
        AgentEventKind::CodingTurn => "coding",
        AgentEventKind::AuditAnchor => "audit",
        AgentEventKind::ContinuationStarted => "continuation started",
        AgentEventKind::ContinuationCompleted => "continuation completed",
        AgentEventKind::ContinuationFailed => "continuation failed",
        AgentEventKind::ContinuationCancelled => "continuation cancelled",
        AgentEventKind::ApprovalRequested => "approval requested",
        AgentEventKind::ApprovalResolved => "approval resolved",
        AgentEventKind::Error => "error",
        AgentEventKind::SessionStart => "session started",
        AgentEventKind::TurnStart => "turn started",
        AgentEventKind::UserMessage => "user message",
        AgentEventKind::TurnEnd => "turn ended",
    }
}

pub(super) fn human_trace_counts(counts: &BTreeMap<&'static str, usize>) -> String {
    let parts = [
        "session",
        "model",
        "tool",
        "context",
        "memory",
        "coding",
        "audit",
        "continuation",
        "approval",
        "error",
    ]
    .into_iter()
    .filter_map(|category| {
        let count = counts.get(category).copied().unwrap_or_default();
        (count > 0).then(|| format!("{category} {count}"))
    })
    .collect::<Vec<_>>();
    if parts.is_empty() {
        "none".into()
    } else {
        parts.join(", ")
    }
}

fn single_line_excerpt(input: &str, max_chars: usize) -> String {
    let normalized = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut output = String::new();
    for (index, ch) in normalized.chars().enumerate() {
        if index >= max_chars {
            output.push_str("...");
            return output;
        }
        output.push(ch);
    }
    if output.is_empty() {
        "none".into()
    } else {
        output
    }
}
