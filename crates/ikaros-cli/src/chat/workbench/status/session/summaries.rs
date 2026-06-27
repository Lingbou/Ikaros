// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_agent::chat::{
    ChatHistorySessionSummary, chat_history_session_summaries_from_session_replays,
};
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::session::SessionStore;
use ikaros_state::session::SqliteSessionStore;
use std::path::Path;

use super::super::super::terminal_inline;
use super::text::truncate_session_text;

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

pub(in crate::chat) fn session_summaries_human_lines(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    limit: usize,
) -> Result<Vec<String>> {
    let sessions = session_replay_history_summaries(config, paths, workspace, runtime, limit)?;
    let mut lines = vec!["* Sessions".to_owned()];
    if sessions.is_empty() {
        lines.push("  no saved sessions".to_owned());
        return Ok(lines);
    }
    for summary in sessions {
        lines.push(format!(
            "  * {} turns={} last={}",
            terminal_inline(&summary.session_id),
            summary.turns,
            terminal_inline(&summary.last_turn_id)
        ));
        lines.push(format!(
            "    * {}",
            truncate_session_text(&summary.last_user_message, 100)
        ));
        lines.push(format!(
            "    * {}",
            truncate_session_text(&summary.last_assistant_message, 120)
        ));
    }
    Ok(lines)
}

fn session_replay_history_summaries(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    limit: usize,
) -> Result<Vec<ChatHistorySessionSummary>> {
    let mut replays = Vec::new();
    for state_db in super::super::state_db_candidates(config, paths, workspace, runtime)? {
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
