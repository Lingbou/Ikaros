// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use crate::chat::workbench::{WorkbenchCell, WorkbenchCellKind, terminal_inline, terminal_message};
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::session::{
    SessionEntry, SessionEntryKind, SessionId, SessionReplay, SessionStore, SqliteSessionStore,
};
use std::path::Path;

use super::super::state_db_candidates;

pub(super) fn screen_conversation_cells(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<Vec<WorkbenchCell>> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    for state_db in state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            return Ok(screen_conversation_cells_from_replay(&replay, 4));
        }
    }
    Ok(Vec::new())
}

fn screen_conversation_cells_from_replay(
    replay: &SessionReplay,
    limit: usize,
) -> Vec<WorkbenchCell> {
    let mut entries = replay
        .entries
        .iter()
        .filter(|entry| {
            matches!(
                entry.kind,
                SessionEntryKind::UserMessage | SessionEntryKind::AssistantMessage
            )
        })
        .rev()
        .take(limit.max(1))
        .collect::<Vec<_>>();
    entries.reverse();
    entries
        .into_iter()
        .map(screen_conversation_cell)
        .collect::<Vec<_>>()
}

fn screen_conversation_cell(entry: &SessionEntry) -> WorkbenchCell {
    let role = match entry.kind {
        SessionEntryKind::AssistantMessage => "assistant",
        SessionEntryKind::UserMessage => "user",
        _ => "entry",
    };
    let text = entry_visible_text(entry);
    let detail = terminal_message(&text);
    WorkbenchCell {
        kind: match entry.kind {
            SessionEntryKind::AssistantMessage => WorkbenchCellKind::Model,
            _ => WorkbenchCellKind::Session,
        },
        title: format!(
            "{} turn={}",
            role,
            entry
                .turn_id
                .as_ref()
                .map(|turn_id| terminal_inline(turn_id.as_str()))
                .unwrap_or_else(|| "none".into())
        ),
        detail,
    }
}

fn entry_visible_text(entry: &SessionEntry) -> String {
    entry
        .visible_text
        .clone()
        .or_else(|| {
            entry
                .payload
                .get("content")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| "none".into())
}
