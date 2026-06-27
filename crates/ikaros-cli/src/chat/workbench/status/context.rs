// SPDX-License-Identifier: GPL-3.0-only

mod cells;
mod output;
mod value;

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_agent::chat::ChatRunOptions;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_state::session::{
    AgentEventKind, SessionId, SessionReplay, SessionStore, SqliteSessionStore,
};
use std::path::Path;

pub(in crate::chat) use output::context_status_human_lines;
use output::{
    context_status_json_line, print_context_engine_registry, print_latest_prompt_sections,
};

use super::super::WorkbenchCell;

pub(in crate::chat) fn print_context_status(
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<()> {
    println!(
        "context_session: {}",
        super::super::terminal_inline(&runtime.chat_session_id)
    );
    println!("context_token_budget: {}", options.context_token_budget);
    println!("context_history_limit: {}", options.history_context_limit);
    println!(
        "context_history_summary_limit: {}",
        options.history_summary_limit
    );
    println!("context_memory_limit: {}", options.memory_limit);
    println!(
        "context_memory_search_limit: {}",
        options.memory_search_limit
    );
    println!("context_rag_top_k: {}", options.rag_top_k);
    println!(
        "context_relationship_learning: {}",
        options.relationship_learning
    );
    println!("context_disabled: {}", options.no_context);
    print_context_engine_registry();
    println!("{}", context_status_json_line(runtime, options)?);
    super::print_filtered_event_cells(runtime, "context", |kind| {
        matches!(
            kind,
            AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted
        )
    })?;
    print_latest_prompt_sections(runtime)?;
    Ok(())
}

pub(in crate::chat) fn print_context_status_for_human(
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<()> {
    for line in context_status_human_lines(runtime, options)? {
        println!("{line}");
    }
    Ok(())
}

pub(super) fn screen_context_cells(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<Vec<WorkbenchCell>> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    for state_db in super::state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            if let Some(cells) = screen_context_cells_from_replay(&replay) {
                return Ok(cells);
            }
            return Ok(cells::screen_context_current_cells(runtime, options));
        }
    }
    Ok(cells::screen_context_current_cells(runtime, options))
}

pub(super) fn screen_context_cells_from_replay(
    replay: &SessionReplay,
) -> Option<Vec<WorkbenchCell>> {
    cells::screen_context_cells_from_replay(replay)
}
