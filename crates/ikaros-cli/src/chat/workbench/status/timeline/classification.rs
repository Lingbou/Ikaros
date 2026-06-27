// SPDX-License-Identifier: GPL-3.0-only

use ikaros_state::session::AgentEventKind;
use std::collections::BTreeMap;

use super::super::super::WorkbenchCellKind;

pub(super) const TIMELINE_CATEGORY_ORDER: &[&str] = &[
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
];

pub(super) fn timeline_category_cell_kind(category: &str) -> WorkbenchCellKind {
    match category {
        "model" => WorkbenchCellKind::Model,
        "tool" => WorkbenchCellKind::Tool,
        "context" => WorkbenchCellKind::Context,
        "memory" => WorkbenchCellKind::Memory,
        "coding" => WorkbenchCellKind::Coding,
        "audit" => WorkbenchCellKind::Audit,
        "continuation" => WorkbenchCellKind::Continuation,
        "approval" => WorkbenchCellKind::Approval,
        "error" => WorkbenchCellKind::Error,
        _ => WorkbenchCellKind::Session,
    }
}
pub(super) fn trace_event_category(kind: &AgentEventKind) -> &'static str {
    match kind {
        AgentEventKind::ModelStream(_) | AgentEventKind::ModelDiagnostic(_) => "model",
        AgentEventKind::ToolCallStarted
        | AgentEventKind::ToolCallOutputDelta
        | AgentEventKind::ToolCallCompleted
        | AgentEventKind::ToolCallFailed
        | AgentEventKind::ToolCallCancelled => "tool",
        AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted => "context",
        AgentEventKind::MemoryLifecycle => "memory",
        AgentEventKind::CodingTurn => "coding",
        AgentEventKind::AuditAnchor => "audit",
        AgentEventKind::ContinuationStarted
        | AgentEventKind::ContinuationCompleted
        | AgentEventKind::ContinuationFailed
        | AgentEventKind::ContinuationCancelled => "continuation",
        AgentEventKind::ApprovalRequested | AgentEventKind::ApprovalResolved => "approval",
        AgentEventKind::Error => "error",
        AgentEventKind::SessionStart
        | AgentEventKind::TurnStart
        | AgentEventKind::UserMessage
        | AgentEventKind::TurnEnd => "session",
    }
}

pub(super) fn format_trace_counts(counts: &BTreeMap<&'static str, usize>) -> String {
    [
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
    .map(|category| format!("{category}={}", counts.get(category).copied().unwrap_or(0)))
    .collect::<Vec<_>>()
    .join(" ")
}
