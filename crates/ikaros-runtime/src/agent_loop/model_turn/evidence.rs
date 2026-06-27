// SPDX-License-Identifier: GPL-3.0-only

use super::events::{emit_agent_event, redacted_json_value};
use crate::agent_loop::types::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopToolResult,
    AgentSessionId, AgentTurnId,
};
use ikaros_core::Result;
use ikaros_harness::{AuditEvent, ExecutionSession};
use ikaros_session::{
    AgentEventId, ApprovalRecord as SessionApprovalRecord, ApprovalStatus as SessionApprovalStatus,
};
use serde_json::json;
use time::OffsetDateTime;

pub(super) fn emit_session_approval_record(
    sink: &dyn AgentEventSink,
    session: &ExecutionSession,
    session_id: &AgentSessionId,
    turn_id: &AgentTurnId,
    output: &serde_json::Value,
) -> Result<()> {
    let Some(approval_id) = output
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(());
    };
    let Some(record) = session.approvals.get(approval_id)? else {
        return Ok(());
    };
    let status = match record.status {
        ikaros_harness::ApprovalStatus::Pending => SessionApprovalStatus::Requested,
        ikaros_harness::ApprovalStatus::Approved => SessionApprovalStatus::Approved,
        ikaros_harness::ApprovalStatus::Denied => SessionApprovalStatus::Denied,
        ikaros_harness::ApprovalStatus::Executed => SessionApprovalStatus::Executed,
    };
    let decision = match record.status {
        ikaros_harness::ApprovalStatus::Pending => None,
        _ => Some(redacted_json_value(json!({
            "status": format!("{:?}", record.status),
            "note": record.note,
            "result": record.result,
        }))),
    };
    sink.emit_approval(&SessionApprovalRecord {
        approval_id: approval_id.into(),
        session_id: session_id.clone(),
        turn_id: Some(turn_id.clone()),
        at: OffsetDateTime::now_utc(),
        status,
        request: redacted_json_value(serde_json::to_value(&record.request)?),
        decision,
    })
}

pub(super) struct ToolAuditAnchorContext<'a> {
    pub(super) session: &'a ExecutionSession,
    pub(super) events: &'a mut Vec<AgentEvent>,
    pub(super) event_sink: &'a dyn AgentEventSink,
    pub(super) session_id: &'a AgentSessionId,
    pub(super) turn_id: &'a AgentTurnId,
    pub(super) iteration: u32,
    pub(super) tool_call_id: &'a Option<String>,
    pub(super) tool_event_id: &'a AgentEventId,
}

pub(super) fn emit_tool_audit_anchor(
    context: ToolAuditAnchorContext<'_>,
    tool_result: &AgentLoopToolResult,
) -> Result<()> {
    let Some(audit_event) = matching_tool_result_audit_event(context.session, tool_result)? else {
        return Ok(());
    };
    let AuditEvent {
        id: audit_event_id,
        kind: audit_kind,
        ..
    } = audit_event;
    emit_agent_event(
        context.events,
        context.event_sink,
        context.session_id,
        context.turn_id,
        AgentEventSource::Audit,
        AgentEventKind::AuditAnchor,
        json!({
            "iteration": context.iteration,
            "tool_call_id": context.tool_call_id,
            "tool_event_id": context.tool_event_id.as_str(),
            "harness_call_id": &tool_result.harness_call_id,
            "approval_id": tool_result
                .output
                .get("approval_id")
                .and_then(serde_json::Value::as_str),
            "name": &tool_result.name,
            "audit_event_id": audit_event_id,
            "audit_kind": audit_kind,
            "audit_path": context.session.audit.path().display().to_string(),
        }),
    )?;
    Ok(())
}

fn matching_tool_result_audit_event(
    session: &ExecutionSession,
    tool_result: &AgentLoopToolResult,
) -> Result<Option<AuditEvent>> {
    let Some(call_id) = tool_result.harness_call_id.as_deref() else {
        return Ok(None);
    };
    Ok(session.audit.read_all()?.into_iter().rev().find(|event| {
        event.kind == "tool_result"
            && event
                .data
                .get("call_id")
                .and_then(serde_json::Value::as_str)
                == Some(call_id)
    }))
}
