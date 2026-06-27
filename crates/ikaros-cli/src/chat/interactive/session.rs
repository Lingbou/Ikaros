// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, anyhow};
use ikaros_core::IkarosConfig;
use ikaros_core::IkarosPaths;
use ikaros_runtime::ChatRunOptions;
use ikaros_session::{SessionBranchSummaryInput, SessionId, SessionStore, SqliteSessionStore};
use serde_json::json;
use std::path::Path;

use super::{InteractiveChatRuntime, terminal_inline};
use crate::chat::workbench::{
    TimelineRequest, TimelineVerbosity, normalize_session_id, print_replay_status,
    print_session_export, print_session_history, print_session_status,
};

pub(super) fn handle_fork_command(
    args: Vec<&str>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    let summary = if args.is_empty() {
        "workbench fork from active leaf".to_owned()
    } else {
        args.join(" ")
    };
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let Some(session) = store.get_session(&session_id)? else {
        println!("session_fork: not_found");
        println!("session: {}", terminal_inline(session_id.as_str()));
        println!("reason: no persisted session timeline found");
        return Ok(());
    };
    let Some(parent_entry_id) = session.active_leaf_entry_id else {
        println!("session_fork: unavailable");
        println!("session: {}", terminal_inline(session_id.as_str()));
        println!("reason: session has no active leaf");
        return Ok(());
    };
    let entry = store.branch_from_entry(&SessionBranchSummaryInput {
        session_id: session_id.clone(),
        parent_entry_id: parent_entry_id.clone(),
        summary: summary.clone(),
        payload: json!({
            "source": "workbench",
            "command": "/fork",
            "agent_id": &runtime.agent_id,
            "workspace": runtime.workspace.display().to_string(),
        }),
    })?;
    println!("session_forked: {}", terminal_inline(session_id.as_str()));
    println!(
        "fork_parent_entry: {}",
        terminal_inline(parent_entry_id.as_str())
    );
    println!("fork_entry: {}", terminal_inline(entry.entry_id.as_str()));
    println!("fork_summary: {}", terminal_inline(&summary));
    Ok(())
}

pub(super) fn handle_session_command(
    args: Vec<&str>,
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &mut InteractiveChatRuntime,
    options: &mut ChatRunOptions,
) -> Result<()> {
    match args.as_slice() {
        [] | ["status"] => {
            print_session_status(config, paths, workspace, runtime, options)?;
        }
        ["resume", session_id] => {
            let session_id = normalize_session_id(session_id);
            if session_id.is_empty() {
                return Err(anyhow!("usage: /session resume <session-id>"));
            }
            runtime.chat_session_id = session_id.clone();
            options.session_id = Some(session_id.clone());
            println!("session_resumed: {}", terminal_inline(&session_id));
        }
        ["history"] => {
            print_session_history(
                config,
                paths,
                workspace,
                runtime,
                &runtime.chat_session_id,
                10,
            )?;
        }
        ["history", limit] => {
            let limit = limit
                .parse::<usize>()
                .with_context(|| "session history limit must be a positive number")?;
            print_session_history(
                config,
                paths,
                workspace,
                runtime,
                &runtime.chat_session_id,
                limit,
            )?;
        }
        ["timeline"] => {
            print_replay_status(
                "timeline",
                config,
                paths,
                workspace,
                runtime,
                TimelineVerbosity::Timeline,
                TimelineRequest::default(),
            )?;
        }
        ["timeline", turn_id] => {
            print_replay_status(
                "timeline",
                config,
                paths,
                workspace,
                runtime,
                TimelineVerbosity::Timeline,
                TimelineRequest::for_turn(turn_id),
            )?;
        }
        ["export"] => {
            print_session_export(config, paths, workspace, runtime, None)?;
        }
        ["export", path] => {
            print_session_export(config, paths, workspace, runtime, Some(path))?;
        }
        _ => {
            println!(
                "usage: /session status|resume <session-id>|history [limit]|timeline [turn]|export [path]"
            );
        }
    }
    Ok(())
}
