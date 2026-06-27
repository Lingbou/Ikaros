// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_session::{SessionEntry, SessionEntryKind, SessionId, SessionStore, SqliteSessionStore};
use serde_json::json;

use super::{InteractiveChatRuntime, terminal_inline};

pub(super) fn append_workbench_evidence(
    runtime: &InteractiveChatRuntime,
    kind: &str,
    payload: serde_json::Value,
) -> Result<()> {
    append_workbench_evidence_with_text(
        runtime,
        kind,
        format!("workbench {kind} status queried"),
        payload,
    )
}

pub(in crate::chat::interactive) fn append_workbench_evidence_with_text(
    runtime: &InteractiveChatRuntime,
    kind: &str,
    visible_text: impl Into<String>,
    payload: serde_json::Value,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let parent_entry_id = store
        .get_session(&session_id)?
        .and_then(|session| session.active_leaf_entry_id);
    let mut entry = SessionEntry::new(session_id.clone(), SessionEntryKind::Custom);
    entry.parent_entry_id = parent_entry_id;
    entry.visible_text = Some(visible_text.into());
    entry.payload = json!({
        "operation": "workbench_evidence",
        "kind": kind,
        "session_id": session_id.as_str(),
        "agent_id": &runtime.agent_id,
        "workspace": runtime.workspace.display().to_string(),
        "data": payload,
    });
    store.append_entry(&entry)?;
    println!(
        "workbench_evidence: kind={} entry={}",
        terminal_inline(kind),
        terminal_inline(entry.entry_id.as_str())
    );
    Ok(())
}
