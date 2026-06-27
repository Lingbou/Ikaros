// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::session::{SessionId, SessionStore, SqliteSessionStore};
use std::path::Path;

use super::super::super::{path_display, terminal_inline};

pub(super) fn print_session_lineage_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    for state_db in super::super::state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(&state_db);
        let Some(session) = store.get_session(&session_id)? else {
            continue;
        };
        let branch = store.active_branch(&session_id)?;
        let continuations = store.continuations(&session_id)?;
        println!("session_state_db: {}", path_display(&state_db));
        println!(
            "session_active_leaf: {}",
            session
                .active_leaf_entry_id
                .as_ref()
                .map(|entry_id| terminal_inline(entry_id.as_str()))
                .unwrap_or_else(|| "none".into())
        );
        println!(
            "session_active_branch_entries: {}",
            branch
                .as_ref()
                .map(|branch| branch.entries.len())
                .unwrap_or_default()
        );
        if let Some(branch) = branch
            && let Some(root) = branch.entries.first()
            && let Some(leaf) = branch.entries.last()
        {
            println!("session_active_branch_root: {}", root.entry_id.as_str());
            println!("session_active_branch_leaf: {}", leaf.entry_id.as_str());
        }
        println!("session_continuations: {}", continuations.len());
        return Ok(());
    }
    println!("session_state_db: none");
    println!("session_active_leaf: none");
    println!("session_active_branch_entries: 0");
    println!("session_continuations: 0");
    Ok(())
}
