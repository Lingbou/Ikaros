// SPDX-License-Identifier: GPL-3.0-only

use crate::resolve_agent_instance;
use ikaros_core::{IkarosConfig, IkarosPaths, Result, redact_json};
use ikaros_execution::harness::{
    ApprovalRecord as HarnessApprovalRecord, ApprovalStatus as HarnessApprovalStatus,
};
use ikaros_state::session::{
    AgentEvent, AgentEventKind, AgentEventSource, ApprovalRecord as SessionApprovalRecord,
    ApprovalStatus as SessionApprovalStatus, SessionStore, SqliteSessionStore,
};
use serde_json::json;
use std::path::Path;

pub fn record_approval_resolution(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    record: &HarnessApprovalRecord,
) -> Result<bool> {
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let store = SqliteSessionStore::new(&agent.state_dir);
    let Some(existing) = store.approval_record(&record.request.id)? else {
        return Ok(false);
    };
    let status = session_approval_status(record.status.clone());
    let decision = if matches!(record.status, HarnessApprovalStatus::Pending) {
        None
    } else {
        Some(redact_json(json!({
            "status": format!("{:?}", record.status),
            "note": &record.note,
            "result": &record.result,
        })))
    };
    let session_id = existing.session_id.clone();
    let event_turn_id = existing.turn_id.clone().unwrap_or_default();
    store.append_approval(&SessionApprovalRecord {
        approval_id: record.request.id.clone(),
        session_id: session_id.clone(),
        turn_id: existing.turn_id.clone(),
        at: time::OffsetDateTime::now_utc(),
        status,
        request: redact_json(serde_json::to_value(&record.request)?),
        decision: decision.clone(),
    })?;
    store.append_agent_event(&AgentEvent::new(
        session_id,
        event_turn_id,
        None,
        AgentEventSource::Harness,
        AgentEventKind::ApprovalResolved,
        json!({
            "approval_id": &record.request.id,
            "status": format!("{:?}", record.status),
            "tool": &record.request.call.name,
            "decision": decision.unwrap_or(serde_json::Value::Null),
        }),
    ))?;
    Ok(true)
}

fn session_approval_status(status: HarnessApprovalStatus) -> SessionApprovalStatus {
    match status {
        HarnessApprovalStatus::Pending => SessionApprovalStatus::Requested,
        HarnessApprovalStatus::Approved => SessionApprovalStatus::Approved,
        HarnessApprovalStatus::Denied => SessionApprovalStatus::Denied,
        HarnessApprovalStatus::Executed => SessionApprovalStatus::Executed,
    }
}
