// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths, redact_json};
use ikaros_state::session::{SessionId, SessionStore, SqliteSessionStore};
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::super::super::{normalize_session_id, path_display, terminal_inline};

pub(in crate::chat) fn print_session_export(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    export_path: Option<&str>,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let candidates = super::super::state_db_candidates(config, paths, workspace, runtime)?;
    for state_db in &candidates {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        let Some(replay) = store.replay_session(&session_id)? else {
            continue;
        };
        let path =
            workbench_session_export_path(paths, workspace, session_id.as_str(), export_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let artifact = serde_json::json!({
            "format": "ikaros-session-export-v1",
            "redacted": true,
            "state_db": state_db.display().to_string(),
            "exported_at": time::OffsetDateTime::now_utc(),
            "session": replay.session,
            "counts": {
                "entries": replay.entries.len(),
                "agent_events": replay.agent_events.len(),
                "approvals": replay.approvals.len(),
            },
            "entries": replay.entries,
            "agent_events": replay.agent_events,
            "approvals": replay.approvals,
        });
        fs::write(&path, serde_json::to_vec_pretty(&redact_json(artifact))?)?;
        println!("session_export: created");
        println!("session: {}", terminal_inline(session_id.as_str()));
        println!("session_export_format: ikaros-session-export-v1");
        println!("session_export_redacted: true");
        println!("session_export_path: {}", path_display(&path));
        println!("state_db: {}", path_display(state_db));
        println!(
            "session_export_counts: entries={} agent_events={} approvals={}",
            replay.entries.len(),
            replay.agent_events.len(),
            replay.approvals.len()
        );
        return Ok(());
    }
    println!("session_export: not_found");
    println!("session: {}", terminal_inline(session_id.as_str()));
    println!("state_db_candidates: {}", candidates.len());
    Ok(())
}

fn workbench_session_export_path(
    paths: &IkarosPaths,
    workspace: &Path,
    session_id: &str,
    export_path: Option<&str>,
) -> PathBuf {
    match export_path.map(str::trim).filter(|path| !path.is_empty()) {
        Some(path) => {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                workspace.join(path)
            }
        }
        None => paths
            .home
            .join("exports")
            .join(format!("session-{}.json", normalize_session_id(session_id))),
    }
}
