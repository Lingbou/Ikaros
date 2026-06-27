// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::attachments::content_block_summary;
use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_agent::chat::ChatRunOptions;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::session::{SessionId, SessionStore, SqliteSessionStore};
use std::path::Path;

use super::super::super::{path_display, terminal_inline};
use super::history_output::session_replay_history_records;
use super::lineage::print_session_lineage_status;

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

pub(in crate::chat) fn session_status_human_lines(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<Vec<String>> {
    let records = session_replay_history_records(
        config,
        paths,
        workspace,
        runtime,
        &runtime.chat_session_id,
    )?
    .unwrap_or_default();
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let state_db = super::super::state_db_candidates(config, paths, workspace, runtime)?
        .into_iter()
        .find(|path| path.exists());
    let (active_leaf, branch_entries, continuations) = if let Some(state_db) = state_db.as_ref() {
        let store = SqliteSessionStore::from_file(state_db);
        let active_leaf = store
            .get_session(&session_id)?
            .and_then(|session| session.active_leaf_entry_id)
            .map(|entry_id| terminal_inline(entry_id.as_str()))
            .unwrap_or_else(|| "none".into());
        let branch_entries = store
            .active_branch(&session_id)?
            .map(|branch| branch.entries.len())
            .unwrap_or_default();
        let continuations = store.continuations(&session_id)?.len();
        (active_leaf, branch_entries, continuations)
    } else {
        ("none".into(), 0, 0)
    };

    let mut lines = vec![
        "* Session".to_owned(),
        format!("  id: {}", terminal_inline(&runtime.chat_session_id)),
        format!("  turns: {}", records.len()),
        format!("  active leaf: {active_leaf}"),
        format!("  branch entries: {branch_entries}"),
        format!("  continuations: {continuations}"),
        format!(
            "  pending attachments: {}",
            runtime.pending_content_blocks.len()
        ),
        "  next turn:".to_owned(),
        format!("    stream: {}", options.stream),
        format!("    agent loop: {}", options.agent_loop),
        format!(
            "    context budget: {} tokens",
            options.context_token_budget
        ),
    ];
    if let Some(state_db) = state_db {
        lines.push(format!("  state: {}", path_display(&state_db)));
    }
    Ok(lines)
}
