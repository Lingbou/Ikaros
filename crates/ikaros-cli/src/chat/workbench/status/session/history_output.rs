// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_agent::chat::{ChatHistoryRecord, chat_history_records_from_session_replay};
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::session::{SessionId, SessionStore, SqliteSessionStore};
use std::path::Path;

use super::super::super::terminal_inline;
use super::text::truncate_session_text;

pub(in crate::chat) fn print_session_history(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    session_id: &str,
    limit: usize,
) -> Result<()> {
    let records = session_replay_history_records(config, paths, workspace, runtime, session_id)?
        .unwrap_or_default();
    println!("session_history: {}", terminal_inline(session_id));
    println!("history_source: session_replay");
    println!("history_authority: session_store");
    println!("records: {}", records.len());
    if records.is_empty() {
        println!("recent: none");
        return Ok(());
    }
    println!("recent:");
    let start = records.len().saturating_sub(limit);
    for record in &records[start..] {
        println!(
            "- turn={} provider={} model={} streamed={} user={} assistant={}",
            terminal_inline(&record.turn_id),
            terminal_inline(&record.provider),
            terminal_inline(&record.model),
            record.streamed,
            terminal_inline(&record.user_message),
            terminal_inline(&record.assistant_message)
        );
    }
    Ok(())
}

pub(in crate::chat) fn session_history_human_lines(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    session_id: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let records = session_replay_history_records(config, paths, workspace, runtime, session_id)?
        .unwrap_or_default();
    let mut lines = vec![
        "* Session History".to_owned(),
        format!("  session: {}", terminal_inline(session_id)),
    ];
    if records.is_empty() {
        lines.push("  no saved turns".to_owned());
        return Ok(lines);
    }
    let start = records.len().saturating_sub(limit);
    for record in &records[start..] {
        lines.push(format!(
            "  * {}",
            truncate_session_text(&record.user_message, 120)
        ));
        lines.push(format!(
            "  * {}",
            truncate_session_text(&record.assistant_message, 160)
        ));
    }
    Ok(lines)
}

pub(super) fn session_replay_history_records(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    session_id: &str,
) -> Result<Option<Vec<ChatHistoryRecord>>> {
    for state_db in super::super::state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(&state_db);
        let Some(replay) = store.replay_session(&SessionId::from(session_id))? else {
            continue;
        };
        let records = chat_history_records_from_session_replay(&replay);
        if !records.is_empty() {
            return Ok(Some(records));
        }
    }
    Ok(None)
}
