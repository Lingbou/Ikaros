// SPDX-License-Identifier: GPL-3.0-only

use ikaros_state::session::{AgentEvent, AgentEventKind};
use ikaros_terminal::{
    ToolActivity, ToolActivityStatus, render_tool_activity as render_tui_tool_activity,
    terminal_inline,
};
use std::env;

pub(in crate::chat) use ikaros_terminal::human_activity_line_for_terminal;

pub(super) fn verbose_live_cells_enabled() -> bool {
    env::var_os("IKAROS_WORKBENCH_VERBOSE_LIVE_CELLS").is_some()
}

fn debug_context_activity_enabled() -> bool {
    verbose_live_cells_enabled() || env::var_os("IKAROS_WORKBENCH_CONTEXT_ACTIVITY").is_some()
}
pub(in crate::chat) fn human_activity_lines(event: &AgentEvent) -> Option<Vec<String>> {
    human_activity_lines_with_debug_context(event, debug_context_activity_enabled())
}

pub(in crate::chat) fn human_activity_lines_with_debug_context(
    event: &AgentEvent,
    include_debug_context: bool,
) -> Option<Vec<String>> {
    match &event.kind {
        AgentEventKind::ToolCallCompleted => {
            let name = event_payload_str(&event.payload, "name").unwrap_or("tool");
            let activity = ToolActivity::new(name, ToolActivityStatus::Completed);
            Some(render_tui_tool_activity(&activity_with_payload_detail(
                activity,
                &event.payload,
            )))
        }
        AgentEventKind::ToolCallFailed => {
            let name = event_payload_str(&event.payload, "name").unwrap_or("tool");
            let activity = ToolActivity::new(name, ToolActivityStatus::Failed);
            Some(render_tui_tool_activity(&activity_with_payload_detail(
                activity,
                &event.payload,
            )))
        }
        AgentEventKind::ToolCallCancelled => {
            let name = event_payload_str(&event.payload, "name").unwrap_or("tool");
            let activity = ToolActivity::new(name, ToolActivityStatus::Cancelled);
            Some(render_tui_tool_activity(&activity))
        }
        AgentEventKind::ContextDiff if include_debug_context => {
            let sections = json_array_len(&event.payload, "sections");
            let references = json_array_len(&event.payload, "references");
            if sections == 0 && references == 0 {
                return None;
            }
            Some(vec![
                "* Gathered context".into(),
                format!("  * {sections} section(s), {references} reference(s)"),
            ])
        }
        AgentEventKind::ContextCompacted if include_debug_context => {
            let sections = json_array_len(&event.payload, "compressed_sections");
            Some(vec![
                "* Compacted context".into(),
                format!("  * {sections} section(s) compressed"),
            ])
        }
        AgentEventKind::CodingTurn => {
            let summary = event_payload_str(&event.payload, "summary")
                .or_else(|| event_payload_str(&event.payload, "status"))
                .unwrap_or("coding workflow updated");
            Some(vec![
                "* Updated code".into(),
                format!("  * {}", terminal_inline(summary)),
            ])
        }
        AgentEventKind::ApprovalRequested => Some(vec!["* Waiting for approval".into()]),
        AgentEventKind::Error => human_error_activity_lines(event),
        _ => None,
    }
}

fn human_error_activity_lines(event: &AgentEvent) -> Option<Vec<String>> {
    if event_payload_str(&event.payload, "phase") == Some("interactive_chat_turn") {
        return None;
    }
    let detail = event_payload_str(&event.payload, "message")
        .or_else(|| event_payload_str(&event.payload, "error"))
        .or_else(|| event_payload_str(&event.payload, "summary"))
        .or_else(|| event_payload_str(&event.payload, "detail"))
        .map(terminal_inline)
        .filter(|message| !message.trim().is_empty());
    let mut lines = vec!["* Error".to_owned()];
    if let Some(detail) = detail {
        lines.push(format!("  * {detail}"));
    }
    Some(lines)
}

fn activity_with_payload_detail(
    activity: ToolActivity,
    payload: &serde_json::Value,
) -> ToolActivity {
    if let Some(summary) = event_payload_str(payload, "summary")
        .or_else(|| event_payload_str(payload, "error"))
        .filter(|summary| !summary.trim().is_empty())
    {
        activity.with_detail(summary)
    } else {
        activity
    }
}

fn event_payload_str<'a>(payload: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(serde_json::Value::as_str)
}

fn json_array_len(payload: &serde_json::Value, key: &str) -> usize {
    payload
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}
