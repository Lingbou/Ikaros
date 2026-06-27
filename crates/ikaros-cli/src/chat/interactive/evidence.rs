// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_agent::chat::workbench_state::append_workbench_evidence_entry;
use ikaros_state::session::{SessionId, SqliteSessionStore};

use super::InteractiveChatRuntime;

pub(super) fn append_workbench_evidence(
    runtime: &InteractiveChatRuntime,
    kind: &str,
    payload: serde_json::Value,
) -> Result<()> {
    if runtime.default_inline_stdout() {
        return Ok(());
    }
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
    if runtime.default_inline_stdout() {
        return Ok(());
    }
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let store = SqliteSessionStore::new(&runtime.state_dir);
    append_workbench_evidence_entry(
        &store,
        &session_id,
        &runtime.agent_id,
        &runtime.workspace,
        kind,
        visible_text,
        payload,
    )?;
    Ok(())
}
