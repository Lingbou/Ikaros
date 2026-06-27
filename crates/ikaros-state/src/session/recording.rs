// SPDX-License-Identifier: GPL-3.0-only

use crate::session::{
    AgentEvent, AgentEventKind, AgentEventSource, SessionEntry, SessionEntryId, SessionEntryKind,
    SessionId, SessionRecord, SessionSource, SessionStore, SqliteSessionStore, TurnId,
};
use ikaros_core::{Result, redact_secrets};
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct RuntimeSessionTarget {
    pub store: SqliteSessionStore,
    pub agent_id: String,
    pub workspace: PathBuf,
}

pub fn upsert_runtime_session(
    target: &RuntimeSessionTarget,
    session_id: &SessionId,
    source: SessionSource,
) -> Result<()> {
    let mut session = SessionRecord::new(session_id.clone(), source);
    session.agent_id = Some(target.agent_id.clone());
    session.workspace = Some(target.workspace.clone());
    target.store.upsert_session(&session)
}

pub fn active_leaf_entry_id(
    target: &RuntimeSessionTarget,
    session_id: &SessionId,
) -> Result<Option<SessionEntryId>> {
    Ok(target
        .store
        .get_session(session_id)?
        .and_then(|session| session.active_leaf_entry_id))
}

pub struct RuntimeSessionEntryInput<'a> {
    pub target: &'a RuntimeSessionTarget,
    pub session_id: &'a SessionId,
    pub parent_entry_id: Option<SessionEntryId>,
    pub turn_id: &'a TurnId,
    pub kind: SessionEntryKind,
    pub visible_text: Option<String>,
    pub payload: serde_json::Value,
}

pub fn append_runtime_session_entry(input: RuntimeSessionEntryInput<'_>) -> Result<SessionEntryId> {
    let mut entry = SessionEntry::new(input.session_id.clone(), input.kind);
    entry.parent_entry_id = input.parent_entry_id;
    entry.turn_id = Some(input.turn_id.clone());
    entry.visible_text = input.visible_text;
    entry.payload = input.payload;
    let entry_id = entry.entry_id.clone();
    input.target.store.append_entry(&entry)?;
    Ok(entry_id)
}

pub fn append_runtime_session_event(
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

pub fn delivery_payload(
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
