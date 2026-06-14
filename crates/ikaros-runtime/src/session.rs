// SPDX-License-Identifier: GPL-3.0-only

use crate::environment::resolve_agent_instance;
use ikaros_core::{IkarosConfig, IkarosPaths, Result, redact_json, redact_secrets};
use ikaros_gateway::{GatewayMessage, GatewayMessageKind};
use ikaros_harness::{
    ApprovalRecord as HarnessApprovalRecord, ApprovalStatus as HarnessApprovalStatus,
};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSource, ApprovalRecord as SessionApprovalRecord,
    ApprovalStatus as SessionApprovalStatus, SessionEntry, SessionEntryId, SessionEntryKind,
    SessionId, SessionRecord, SessionSource, SessionStore, SqliteSessionStore, TurnId,
};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub(crate) struct RuntimeSessionTarget {
    pub store: SqliteSessionStore,
    pub agent_id: String,
    pub workspace: PathBuf,
}

pub(crate) fn runtime_session_target(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<RuntimeSessionTarget> {
    let config = IkarosConfig::load(&paths.config)?;
    let agent_instance = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    Ok(RuntimeSessionTarget {
        store: SqliteSessionStore::new(&agent_instance.state_dir),
        agent_id: agent_instance.agent_id,
        workspace: agent_instance.workspace,
    })
}

pub(crate) fn upsert_runtime_session(
    target: &RuntimeSessionTarget,
    session_id: &SessionId,
    source: SessionSource,
) -> Result<()> {
    let mut session = SessionRecord::new(session_id.clone(), source);
    session.agent_id = Some(target.agent_id.clone());
    session.workspace = Some(target.workspace.clone());
    target.store.upsert_session(&session)
}

pub(crate) fn active_leaf_entry_id(
    target: &RuntimeSessionTarget,
    session_id: &SessionId,
) -> Result<Option<SessionEntryId>> {
    Ok(target
        .store
        .get_session(session_id)?
        .and_then(|session| session.active_leaf_entry_id))
}

pub(crate) struct RuntimeSessionEntryInput<'a> {
    pub target: &'a RuntimeSessionTarget,
    pub session_id: &'a SessionId,
    pub parent_entry_id: Option<SessionEntryId>,
    pub turn_id: &'a TurnId,
    pub kind: SessionEntryKind,
    pub visible_text: Option<String>,
    pub payload: serde_json::Value,
}

pub(crate) fn append_runtime_session_entry(
    input: RuntimeSessionEntryInput<'_>,
) -> Result<SessionEntryId> {
    let mut entry = SessionEntry::new(input.session_id.clone(), input.kind);
    entry.parent_entry_id = input.parent_entry_id;
    entry.turn_id = Some(input.turn_id.clone());
    entry.visible_text = input.visible_text;
    entry.payload = input.payload;
    let entry_id = entry.entry_id.clone();
    input.target.store.append_entry(&entry)?;
    Ok(entry_id)
}

pub(crate) fn append_runtime_session_event(
    target: &RuntimeSessionTarget,
    session_id: &SessionId,
    turn_id: &TurnId,
    source: AgentEventSource,
    kind: AgentEventKind,
    payload: serde_json::Value,
) -> Result<()> {
    target.store.append_agent_event(&AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        source,
        kind,
        payload,
    ))
}

pub(crate) fn gateway_session_id(message: &GatewayMessage) -> SessionId {
    let channel = message
        .session_source
        .as_ref()
        .map(|source| source.channel.as_str())
        .unwrap_or(message.source.as_str());
    let account = message
        .session_source
        .as_ref()
        .and_then(|source| source.account.as_deref())
        .unwrap_or("_");
    let peer = message
        .session_source
        .as_ref()
        .and_then(|source| source.peer.as_deref())
        .unwrap_or("_");
    let thread = message
        .session_source
        .as_ref()
        .and_then(|source| source.thread.as_deref())
        .or_else(|| {
            message
                .session_source
                .as_ref()
                .and_then(|source| source.message_id.as_deref())
        })
        .unwrap_or(message.id.as_str());
    SessionId::from(format!(
        "gateway:{}:{}:{}:{}",
        redact_session_segment(channel),
        redact_session_segment(account),
        redact_session_segment(peer),
        redact_session_segment(thread)
    ))
}

pub(crate) fn gateway_session_source(message: &GatewayMessage) -> SessionSource {
    let source = message.session_source.as_ref();
    SessionSource::Gateway {
        channel: redact_secrets(
            source
                .map(|source| source.channel.as_str())
                .unwrap_or(message.source.as_str()),
        ),
        account: source
            .and_then(|source| source.account.as_deref())
            .map(redact_secrets),
        peer: source
            .and_then(|source| source.peer.as_deref())
            .map(redact_secrets),
        thread: source
            .and_then(|source| source.thread.as_deref())
            .map(redact_secrets),
        message_id: source
            .and_then(|source| source.message_id.as_deref())
            .map(redact_secrets)
            .or_else(|| Some(redact_secrets(&message.id))),
    }
}

pub(crate) fn gateway_turn_id(message_id: &str) -> TurnId {
    TurnId::from(format!("gateway-{message_id}"))
}

pub(crate) fn gateway_message_kind(kind: &GatewayMessageKind) -> &'static str {
    match kind {
        GatewayMessageKind::Chat => "chat",
        GatewayMessageKind::Task => "task",
    }
}

pub(crate) fn schedule_session_id(job_id: &str) -> SessionId {
    SessionId::from(format!("schedule:{}", redact_session_segment(job_id)))
}

pub(crate) fn schedule_turn_id(run_id: &str) -> TurnId {
    TurnId::from(format!("schedule-{run_id}"))
}

pub(crate) fn schedule_session_source(job_id: &str) -> SessionSource {
    SessionSource::Schedule {
        job_id: redact_secrets(job_id),
    }
}

pub fn record_approval_resolution(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    record: &HarnessApprovalRecord,
) -> Result<bool> {
    let target = runtime_session_target(paths, workspace, agent_override)?;
    let Some(existing) = target.store.approval_record(&record.request.id)? else {
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
    target.store.append_approval(&SessionApprovalRecord {
        approval_id: record.request.id.clone(),
        session_id: existing.session_id.clone(),
        turn_id: existing.turn_id.clone(),
        at: time::OffsetDateTime::now_utc(),
        status,
        request: redact_json(serde_json::to_value(&record.request)?),
        decision: decision.clone(),
    })?;
    if let Some(turn_id) = existing.turn_id {
        target.store.append_agent_event(&AgentEvent::new(
            existing.session_id,
            turn_id,
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
    }
    Ok(true)
}

pub(crate) fn delivery_payload(
    kind: &str,
    status: &str,
    summary: &str,
    delivery: Option<serde_json::Value>,
) -> serde_json::Value {
    json!({
        "role": "runtime",
        "kind": kind,
        "status": status,
        "summary": redact_secrets(summary),
        "delivery": delivery.unwrap_or(serde_json::Value::Null),
    })
}

fn redact_session_segment(value: &str) -> String {
    redact_secrets(value).replace(['/', '\\', ':', '\n', '\r', '\t'], "_")
}

fn session_approval_status(status: HarnessApprovalStatus) -> SessionApprovalStatus {
    match status {
        HarnessApprovalStatus::Pending => SessionApprovalStatus::Requested,
        HarnessApprovalStatus::Approved => SessionApprovalStatus::Approved,
        HarnessApprovalStatus::Denied => SessionApprovalStatus::Denied,
        HarnessApprovalStatus::Executed => SessionApprovalStatus::Executed,
    }
}
