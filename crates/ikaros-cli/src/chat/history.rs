// SPDX-License-Identifier: GPL-3.0-only

use crate::resolve_agent_instance;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths, redact_secrets};
use ikaros_runtime::{
    CHAT_HISTORY_DELETE_SESSION_OPERATION, ChatHistoryRecord, ChatHistorySessionSummary,
    chat_history_records_from_session_replay, chat_history_session_summaries_from_session_replays,
    search_chat_history_records,
};
use ikaros_session::{SessionEntry, SessionEntryKind, SessionId, SessionStore, SqliteSessionStore};
use serde_json::json;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

pub(super) fn print_chat_history(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    limit: usize,
    session_id: Option<&str>,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let records = if let Some(session_id) = session_id {
        session_replay_chat_history_records(&config, paths, workspace, agent_override, session_id)?
            .unwrap_or_default()
    } else {
        session_replay_all_chat_history_records(&config, paths, workspace, agent_override)?
            .unwrap_or_default()
    };
    println!("history_source: session_replay");
    println!("history_authority: session_store");
    if let Some(session_id) = session_id {
        println!("session: {session_id}");
    }
    println!("records: {}", records.len());
    if records.is_empty() {
        println!("recent: none");
        return Ok(());
    }
    let start = records.len().saturating_sub(limit);
    print_chat_history_records("recent", &records[start..]);
    Ok(())
}

fn session_replay_chat_history_records(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: &str,
) -> Result<Option<Vec<ChatHistoryRecord>>> {
    for state_db in chat_state_db_candidates(config, paths, workspace, agent_override)? {
        if !state_db.is_file() {
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

fn session_replay_all_chat_history_records(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Option<Vec<ChatHistoryRecord>>> {
    let mut records = Vec::new();
    for state_db in chat_state_db_candidates(config, paths, workspace, agent_override)? {
        if !state_db.is_file() {
            continue;
        }
        let store = SqliteSessionStore::from_file(&state_db);
        for session in store.session_records()? {
            let Some(replay) = store.replay_session(&session.session_id)? else {
                continue;
            };
            records.extend(chat_history_records_from_session_replay(&replay));
        }
    }
    if records.is_empty() {
        return Ok(None);
    }
    records.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
            .then_with(|| left.turn_id.cmp(&right.turn_id))
    });
    Ok(Some(records))
}

fn chat_state_db_candidates(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    let agent = resolve_agent_instance(config, agent_override, workspace, &paths.home)?;
    push_chat_state_db_candidate(&mut candidates, &mut seen, agent.state_dir.join("state.db"));
    let agents_dir = paths.home.join("agents");
    if agents_dir.is_dir() {
        for entry in fs::read_dir(&agents_dir)? {
            let entry = entry?;
            push_chat_state_db_candidate(&mut candidates, &mut seen, entry.path().join("state.db"));
        }
    }
    Ok(candidates)
}

fn push_chat_state_db_candidate(
    candidates: &mut Vec<PathBuf>,
    seen: &mut BTreeSet<PathBuf>,
    candidate: PathBuf,
) {
    if seen.insert(candidate.clone()) {
        candidates.push(candidate);
    }
}

pub(super) fn search_chat_history(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    query: &str,
    limit: usize,
    session_id: Option<&str>,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let records = if let Some(session_id) = session_id {
        if let Some(replay_records) = session_replay_chat_history_records(
            &config,
            paths,
            workspace,
            agent_override,
            session_id,
        )? {
            search_chat_history_records(replay_records, query, limit, Some(session_id))
        } else {
            Vec::new()
        }
    } else if let Some(replay_records) =
        session_replay_all_chat_history_records(&config, paths, workspace, agent_override)?
    {
        search_chat_history_records(replay_records, query, limit, None)
    } else {
        Vec::new()
    };
    println!("history_source: session_replay");
    println!("history_authority: session_store");
    println!("query: {}", redact_secrets(query));
    if let Some(session_id) = session_id {
        println!("session: {session_id}");
    }
    println!("records: {}", records.len());
    if records.is_empty() {
        println!("matches: none");
        return Ok(());
    }
    print_chat_history_records("matches", &records);
    Ok(())
}

fn print_chat_history_records(label: &str, records: &[ChatHistoryRecord]) {
    println!("{label}:");
    for record in records {
        println!(
            "- {} session={} turn={} agent={} provider={} model={} streamed={} context=relationship:{} memory:{} rag:{}",
            record.created_at,
            record.session_id,
            record.turn_id,
            record.agent,
            record.provider,
            record.model,
            record.streamed,
            record.relationship_hits,
            record.memory_hits,
            record.rag_hits
        );
        println!("  user: {}", record.user_message);
        println!("  assistant: {}", record.assistant_message);
    }
}

pub(super) fn print_chat_sessions(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    limit: usize,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let sessions =
        session_replay_chat_session_summaries(&config, paths, workspace, agent_override, limit)?;
    println!("history_source: session_replay");
    println!("history_authority: session_store");
    println!("sessions: {}", sessions.len());
    if sessions.is_empty() {
        println!("recent: none");
        return Ok(());
    }
    print_chat_session_summaries(&sessions);
    Ok(())
}

fn session_replay_chat_session_summaries(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    limit: usize,
) -> Result<Vec<ChatHistorySessionSummary>> {
    let mut replays = Vec::new();
    for state_db in chat_state_db_candidates(config, paths, workspace, agent_override)? {
        if !state_db.is_file() {
            continue;
        }
        let store = SqliteSessionStore::from_file(&state_db);
        for session in store.session_records()? {
            if let Some(replay) = store.replay_session(&session.session_id)? {
                replays.push(replay);
            }
        }
    }
    Ok(chat_history_session_summaries_from_session_replays(
        &replays, limit,
    ))
}

fn print_chat_session_summaries(sessions: &[ChatHistorySessionSummary]) {
    println!("recent:");
    for session in sessions {
        println!(
            "- session={} turns={} first={} last={} last_turn={} agents={} providers={} models={}",
            session.session_id,
            session.turns,
            session.first_created_at,
            session.last_created_at,
            session.last_turn_id,
            session.agents.join(","),
            session.providers.join(","),
            session.models.join(",")
        );
        println!("  last_user: {}", session.last_user_message);
        println!("  last_assistant: {}", session.last_assistant_message);
        println!(
            "  continue: ikaros chat --chat-session {} --message \"...\"",
            session.session_id
        );
    }
}

pub(super) fn delete_chat_history_session(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: &str,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let replay_deleted = mark_chat_history_session_deleted(
        &config,
        paths,
        workspace,
        agent_override,
        session_id,
        "history_delete_session",
    )?;
    println!("history_source: session_replay");
    println!("history_authority: session_store");
    println!("deleted_session: {session_id}");
    println!("deleted_records: 0");
    println!("deleted_session_replay: {replay_deleted}");
    Ok(())
}

pub(super) fn clear_chat_history(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let replay_deleted =
        mark_all_chat_history_sessions_deleted(&config, paths, workspace, agent_override)?;
    println!("history_source: session_replay");
    println!("history_authority: session_store");
    println!("deleted_records: 0");
    println!("deleted_session_replay_sessions: {replay_deleted}");
    Ok(())
}

fn mark_chat_history_session_deleted(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: &str,
    reason: &str,
) -> Result<bool> {
    for state_db in chat_state_db_candidates(config, paths, workspace, agent_override)? {
        if !state_db.is_file() {
            continue;
        }
        let store = SqliteSessionStore::from_file(&state_db);
        let Some(replay) = store.replay_session(&SessionId::from(session_id))? else {
            continue;
        };
        append_chat_history_delete_tombstone(&store, &replay, reason)?;
        return Ok(true);
    }
    Ok(false)
}

fn mark_all_chat_history_sessions_deleted(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<usize> {
    let mut deleted = 0usize;
    for state_db in chat_state_db_candidates(config, paths, workspace, agent_override)? {
        if !state_db.is_file() {
            continue;
        }
        let store = SqliteSessionStore::from_file(&state_db);
        for session in store.session_records()? {
            let Some(replay) = store.replay_session(&session.session_id)? else {
                continue;
            };
            if chat_history_records_from_session_replay(&replay).is_empty() {
                continue;
            }
            append_chat_history_delete_tombstone(&store, &replay, "history_clear")?;
            deleted += 1;
        }
    }
    Ok(deleted)
}

fn append_chat_history_delete_tombstone(
    store: &SqliteSessionStore,
    replay: &ikaros_session::SessionReplay,
    reason: &str,
) -> Result<()> {
    let mut entry = SessionEntry::new(replay.session.session_id.clone(), SessionEntryKind::Custom);
    entry.parent_entry_id = replay.entries.last().map(|entry| entry.entry_id.clone());
    entry.turn_id = replay
        .entries
        .iter()
        .rev()
        .find_map(|entry| entry.turn_id.clone());
    entry.visible_text = Some("chat history hidden from history projection".into());
    entry.payload = json!({
        "operation": CHAT_HISTORY_DELETE_SESSION_OPERATION,
        "reason": reason,
    });
    Ok(store.append_entry(&entry)?)
}
