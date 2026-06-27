// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::attachments::content_block_summary;
use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths, redact_json};
use ikaros_runtime::{
    ChatHistoryRecord, ChatHistorySessionSummary, ChatRunOptions,
    chat_history_records_from_session_replay, chat_history_session_summaries_from_session_replays,
};
use ikaros_session::{SessionId, SessionStore, SqliteSessionStore};
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::super::{normalize_session_id, path_display, terminal_inline};

pub(in crate::chat) fn print_session_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<()> {
    let (records, source) = session_replay_history_records(
        config,
        paths,
        workspace,
        runtime,
        &runtime.chat_session_id,
    )?
    .map(|records| (records, "session_store"))
    .unwrap_or_else(|| (Vec::new(), "session_store"));
    println!(
        "workbench_session: {}",
        terminal_inline(&runtime.chat_session_id)
    );
    println!(
        "session_history_records: {} source={}",
        records.len(),
        source
    );
    println!(
        "session_options: agent_loop={} effective_agent_loop={} stream={} no_context={} context_token_budget={} content_blocks={} scope={}",
        options.agent_loop,
        options.agent_loop && options.content_blocks.is_empty(),
        options.stream,
        options.no_context,
        options.context_token_budget,
        options.content_blocks.len(),
        options
            .scope
            .as_deref()
            .map(terminal_inline)
            .unwrap_or_else(|| "none".into())
    );
    println!(
        "session_attachments: pending={} next_turn_agent_loop={}",
        runtime.pending_content_blocks.len(),
        options.agent_loop
            && options.content_blocks.is_empty()
            && runtime.pending_content_blocks.is_empty()
    );
    for (index, block) in runtime.pending_content_blocks.iter().enumerate() {
        println!(
            "session_attachment {}: {}",
            index + 1,
            terminal_inline(&content_block_summary(block))
        );
    }
    print_session_lineage_status(config, paths, workspace, runtime)?;
    Ok(())
}

fn print_session_lineage_status(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    for state_db in super::state_db_candidates(config, paths, workspace, runtime)? {
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

fn session_replay_history_records(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    session_id: &str,
) -> Result<Option<Vec<ChatHistoryRecord>>> {
    for state_db in super::state_db_candidates(config, paths, workspace, runtime)? {
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

pub(in crate::chat) fn print_session_summaries(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    limit: usize,
) -> Result<()> {
    let replay_sessions =
        session_replay_history_summaries(config, paths, workspace, runtime, limit)?;
    let sessions = replay_sessions;
    println!("sessions: {}", sessions.len());
    println!("history_source: session_replay");
    println!("history_authority: session_store");
    if sessions.is_empty() {
        println!("recent: none");
        return Ok(());
    }
    for summary in sessions {
        print_session_summary_line(&summary);
    }
    Ok(())
}

fn session_replay_history_summaries(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    limit: usize,
) -> Result<Vec<ChatHistorySessionSummary>> {
    let mut replays = Vec::new();
    for state_db in super::state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
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

fn print_session_summary_line(summary: &ChatHistorySessionSummary) {
    println!(
        "- session={} turns={} first={} last={} last_turn={} agents={} providers={} models={}",
        terminal_inline(&summary.session_id),
        summary.turns,
        summary.first_created_at,
        summary.last_created_at,
        summary.last_turn_id,
        terminal_inline(&summary.agents.join(",")),
        terminal_inline(&summary.providers.join(",")),
        terminal_inline(&summary.models.join(","))
    );
    println!(
        "  last_user: {}",
        terminal_inline(&summary.last_user_message)
    );
    println!(
        "  last_assistant: {}",
        terminal_inline(&summary.last_assistant_message)
    );
    println!(
        "  continue: ikaros chat --chat-session {} --message \"...\"",
        terminal_inline(&summary.session_id)
    );
}

pub(in crate::chat) fn print_session_export(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    export_path: Option<&str>,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let candidates = super::state_db_candidates(config, paths, workspace, runtime)?;
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
