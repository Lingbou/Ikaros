// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::{
    dispatch::tool_result_cancelled,
    types::{AgentEventKind, AgentLoopToolResult},
};
use ikaros_core::redact_secrets;
use serde_json::json;

pub(super) fn attach_recoverable_tool_retry(
    result: &mut AgentLoopToolResult,
    tool_name: &str,
    tool_input: serde_json::Value,
) {
    if !result.recoverable {
        return;
    }
    let retry = json!({
        "tool_name": redact_secrets(tool_name),
        "tool_input": tool_input,
    });
    match &mut result.output {
        serde_json::Value::Object(output) => {
            output.entry("retry").or_insert(retry);
        }
        output => {
            let original = std::mem::take(output);
            *output = json!({
                "result": original,
                "retry": retry,
            });
        }
    }
}

pub(super) fn tool_lifecycle_end_kind(result: &AgentLoopToolResult) -> AgentEventKind {
    if tool_result_cancelled(result) {
        AgentEventKind::ToolCallCancelled
    } else if result.ok || tool_result_waiting_for_approval(result) {
        AgentEventKind::ToolCallCompleted
    } else {
        AgentEventKind::ToolCallFailed
    }
}

pub(super) fn tool_lifecycle_status(result: &AgentLoopToolResult) -> &'static str {
    if tool_result_cancelled(result) {
        "cancelled"
    } else if result.ok {
        "completed"
    } else if tool_result_waiting_for_approval(result) {
        "waiting_for_approval"
    } else {
        "failed"
    }
}

pub(super) fn tool_result_waiting_for_approval(result: &AgentLoopToolResult) -> bool {
    result.output.get("approval_id").is_some()
        || result
            .output
            .get("decision")
            .and_then(serde_json::Value::as_str)
            == Some("ask_user")
}
