// SPDX-License-Identifier: GPL-3.0-only

use super::types::{AgentLoopStopReason, AgentLoopToolCall, AgentLoopToolResult};
use ikaros_core::{Result, ToolResult, redact_json, redact_secrets};
use ikaros_harness::{
    AuditEvent, ExecutionSession, GuardrailConfig, GuardrailDecision, GuardrailObservation,
    GuardrailSignal, GuardrailState, SkillRegistry,
};
use ikaros_models::ModelMessage;
use serde_json::json;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tokio::time::{Duration, timeout};

pub(super) async fn dispatch_agent_loop_tool_call(
    session: &ExecutionSession,
    registry: &SkillRegistry,
    iteration: u32,
    call: AgentLoopToolCall,
    timeout_ms: Option<u64>,
) -> AgentLoopToolResult {
    let name = redact_secrets(&call.name);
    let started_at = OffsetDateTime::now_utc();
    let execution = session.execute_skill(registry, &call.name, call.input.clone());
    let result = match timeout_ms {
        Some(timeout_ms) if timeout_ms > 0 => {
            match timeout(Duration::from_millis(timeout_ms), execution).await {
                Ok(result) => result,
                Err(_) => {
                    return timed_out_agent_loop_tool_result(
                        iteration, name, timeout_ms, started_at,
                    );
                }
            }
        }
        _ => execution.await,
    };
    match result {
        Ok(result) => agent_loop_tool_result_from_tool_result(iteration, name, result),
        Err(error) => AgentLoopToolResult {
            iteration,
            name,
            harness_call_id: None,
            ok: false,
            summary: redact_secrets(&error.to_string()),
            output: json!({"error": redact_secrets(&error.to_string())}),
            recoverable: false,
        },
    }
}

fn timed_out_agent_loop_tool_result(
    iteration: u32,
    name: String,
    timeout_ms: u64,
    started_at: OffsetDateTime,
) -> AgentLoopToolResult {
    let ended_at = OffsetDateTime::now_utc();
    let summary = format!("tool {name} timed out after {timeout_ms} ms");
    AgentLoopToolResult {
        iteration,
        name,
        harness_call_id: None,
        ok: false,
        summary: redact_secrets(&summary),
        output: json!({
            "error": redact_secrets(&summary),
            "timeout": {
                "kind": "tool",
                "reason": "tool_timeout",
                "timeout_ms": timeout_ms,
                "started_at": format_rfc3339(started_at),
                "ended_at": format_rfc3339(ended_at),
            }
        }),
        recoverable: true,
    }
}

fn format_rfc3339(value: OffsetDateTime) -> String {
    value.format(&Rfc3339).unwrap_or_else(|_| value.to_string())
}

pub(super) fn observe_agent_loop_tool_result(
    session: &ExecutionSession,
    task_id: Option<&str>,
    guardrails: &mut GuardrailState,
    config: &GuardrailConfig,
    result: &AgentLoopToolResult,
) -> Result<Option<AgentLoopStopReason>> {
    if !should_observe_agent_loop_result(result) {
        return Ok(None);
    }
    let observation = if result.ok {
        GuardrailObservation::tool(&result.name, result.ok, &result.summary, &result.output)
    } else {
        GuardrailObservation::failure(&result.name, &result.summary)
    };
    match guardrails.observe(config, &observation) {
        GuardrailDecision::Continue => Ok(None),
        GuardrailDecision::Warn(signal) => {
            audit_agent_loop_guardrail(session, task_id, &signal, false)?;
            Ok(None)
        }
        GuardrailDecision::Halt(signal) => {
            audit_agent_loop_guardrail(session, task_id, &signal, true)?;
            Ok(Some(AgentLoopStopReason::GuardrailHalt))
        }
    }
}

pub(super) fn stop_reason_from_tool_result(
    result: &AgentLoopToolResult,
) -> Option<AgentLoopStopReason> {
    match result
        .output
        .get("decision")
        .and_then(serde_json::Value::as_str)
    {
        Some("deny") => Some(AgentLoopStopReason::PolicyDenied),
        Some("ask_user") => Some(AgentLoopStopReason::WaitingForApproval),
        _ if result.output.get("approval_id").is_some() => {
            Some(AgentLoopStopReason::WaitingForApproval)
        }
        _ if tool_result_cancelled(result) => Some(AgentLoopStopReason::Cancelled),
        _ => None,
    }
}

pub(super) fn tool_result_cancelled(result: &AgentLoopToolResult) -> bool {
    result
        .output
        .get("cancelled")
        .and_then(serde_json::Value::as_bool)
        == Some(true)
}

pub(super) fn model_message_for_tool_result(
    tool_call_id: Option<String>,
    tool_call_name: String,
    result: &AgentLoopToolResult,
) -> ModelMessage {
    let content = render_tool_result_message(result);
    match tool_call_id {
        Some(tool_call_id) if !tool_call_id.trim().is_empty() => {
            ModelMessage::tool_result(tool_call_id, tool_call_name, content)
        }
        _ => ModelMessage::user(content),
    }
}

fn agent_loop_tool_result_from_tool_result(
    iteration: u32,
    name: String,
    result: ToolResult,
) -> AgentLoopToolResult {
    AgentLoopToolResult {
        iteration,
        name,
        harness_call_id: Some(result.call_id),
        ok: result.ok,
        summary: redact_secrets(&result.summary),
        output: redact_json(result.output),
        recoverable: false,
    }
}

fn audit_agent_loop_guardrail(
    session: &ExecutionSession,
    task_id: Option<&str>,
    signal: &GuardrailSignal,
    halted: bool,
) -> Result<()> {
    let kind = if halted {
        "agent_loop_guardrail_halt"
    } else {
        "agent_loop_guardrail_warning"
    };
    session
        .audit
        .append(session.correlate_audit_event(AuditEvent::new(
            kind,
            None,
            signal.message(),
            json!({
                "correlation_id": session.correlation_id(),
                "task_id": task_id,
                "signal": signal,
                "halted": halted,
            }),
        )?))
}

fn should_observe_agent_loop_result(result: &AgentLoopToolResult) -> bool {
    !tool_result_cancelled(result)
        && (result.ok
            || (result.output.get("approval_id").is_none()
                && result.output.get("decision").is_none()))
}

fn render_tool_result_message(result: &AgentLoopToolResult) -> String {
    redact_secrets(&format!(
        "Tool result for {}: ok={} summary={} output={}",
        result.name, result.ok, result.summary, result.output
    ))
}
