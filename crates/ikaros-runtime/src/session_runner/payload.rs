// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::AgentLoopToolResult;
use ikaros_core::{IkarosError, Result, ToolResult, redact_json, redact_secrets};
use ikaros_session::SessionContinuation;

pub(super) fn tool_result_continuation_payload(
    result: &ToolResult,
    started_at: time::OffsetDateTime,
    ended_at: time::OffsetDateTime,
) -> serde_json::Value {
    serde_json::json!({
        "call_id": &result.call_id,
        "ok": result.ok,
        "summary": redact_secrets(&result.summary),
        "output": redact_json(result.output.clone()),
        "started_at": started_at.to_string(),
        "ended_at": ended_at.to_string(),
    })
}

pub(super) fn failed_tool_result_continuation_payload(
    error: &str,
    started_at: time::OffsetDateTime,
    ended_at: time::OffsetDateTime,
) -> serde_json::Value {
    serde_json::json!({
        "ok": false,
        "summary": "tool execution failed",
        "error": redact_secrets(error),
        "started_at": started_at.to_string(),
        "ended_at": ended_at.to_string(),
    })
}

pub(super) fn recoverable_tool_result_retry(
    result: &AgentLoopToolResult,
) -> Option<(String, serde_json::Value)> {
    if !result.recoverable {
        return None;
    }
    let retry = result.output.get("retry")?;
    let tool_name = retry
        .get("tool_name")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .unwrap_or(&result.name)
        .to_owned();
    let tool_input = retry.get("tool_input")?.clone();
    Some((tool_name, tool_input))
}

pub(super) fn continuation_payload_str(
    continuation: &SessionContinuation,
    key: &str,
) -> Result<String> {
    continuation
        .payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            IkarosError::Message(format!(
                "continuation {} missing string payload field {key}",
                continuation.continuation_id
            ))
        })
}
