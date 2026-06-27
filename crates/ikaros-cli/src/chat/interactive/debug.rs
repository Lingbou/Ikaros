// SPDX-License-Identifier: GPL-3.0-only

use super::{
    InteractiveChatRuntime, parse_debug_dump_recent_logs, parse_debug_logs_args,
    parse_timeline_request, print_workbench_continuation_status,
};
use crate::chat::workbench::{
    TimelineVerbosity, print_replay_status, print_replay_status_for_human,
};
use crate::debug::{
    debug_dump_json_line, debug_insights_json_line, debug_logs_json_line,
    debug_memory_lifecycle_json_line, debug_readiness_json_line, debug_sandbox_json_line,
    debug_state_db_json_line,
};
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_terminal::terminal_inline;
use std::path::Path;

pub(super) async fn handle_debug_command(
    args: Vec<&str>,
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    if runtime.default_inline_stdout() {
        print_debug_status_for_human(&args);
    } else if args.first().copied() == Some("readiness") {
        println!("readiness: see readiness_json for structured MVP status");
        println!(
            "{}",
            debug_readiness_json_line(paths, workspace, Some(&runtime.agent.name))?
        );
    } else if args.first().copied() == Some("sandbox") {
        let probe = args.get(1).copied() == Some("--probe");
        println!(
            "{}",
            debug_sandbox_json_line(paths, workspace, Some(&runtime.agent.name), probe).await?
        );
    } else if args.first().copied() == Some("logs") {
        let (source, page, page_size) = parse_debug_logs_args(&args[1..])?;
        println!("{}", debug_logs_json_line(paths, source, page, page_size)?);
    } else if args.first().copied() == Some("insights") {
        println!(
            "{}",
            debug_insights_json_line(paths, workspace, Some(&runtime.agent.name))?
        );
    } else if args.first().copied() == Some("dump") {
        let recent_logs = parse_debug_dump_recent_logs(&args[1..])?;
        println!(
            "{}",
            debug_dump_json_line(paths, workspace, Some(&runtime.agent.name), recent_logs)?
        );
    } else if args.first().copied() == Some("state-db") || args.first().copied() == Some("state_db")
    {
        println!(
            "{}",
            debug_state_db_json_line(paths, workspace, Some(&runtime.agent.name))?
        );
    } else if args.first().copied() == Some("continuations") {
        print_workbench_continuation_status(runtime)?;
    } else if args.first().copied() == Some("memory-lifecycle")
        || args.first().copied() == Some("memory_lifecycle")
    {
        let (session_id, turn_id) =
            parse_debug_memory_lifecycle_args(&args[1..], &runtime.chat_session_id);
        println!(
            "{}",
            debug_memory_lifecycle_json_line(
                paths,
                workspace,
                Some(&runtime.agent.name),
                &session_id,
                turn_id.as_deref(),
            )?
        );
    } else if runtime.default_inline_stdout() {
        print_replay_status_for_human(
            "debug",
            config,
            paths,
            workspace,
            runtime,
            TimelineVerbosity::Debug,
            parse_timeline_request(args)?,
        )?;
    } else {
        print_replay_status(
            "debug",
            config,
            paths,
            workspace,
            runtime,
            TimelineVerbosity::Debug,
            parse_timeline_request(args)?,
        )?;
    }
    Ok(())
}

pub(in crate::chat) fn print_debug_status_for_human(args: &[&str]) {
    for line in debug_status_human_lines(args) {
        println!("{line}");
    }
}

fn parse_debug_memory_lifecycle_args(
    args: &[&str],
    default_session_id: &str,
) -> (String, Option<String>) {
    let mut session_id = default_session_id.to_owned();
    let mut turn_id = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--turn-id" => {
                if let Some(value) = args.get(index + 1) {
                    turn_id = Some((*value).to_owned());
                    index += 2;
                } else {
                    index += 1;
                }
            }
            value if !value.starts_with('-') => {
                session_id = value.to_owned();
                index += 1;
            }
            _ => index += 1,
        }
    }
    (session_id, turn_id)
}

fn debug_status_human_lines(args: &[&str]) -> Vec<String> {
    let mut lines = vec!["* Debug".to_owned()];
    match args.first().copied() {
        Some("readiness") => {
            lines.push(
                "  readiness: use `ikaros debug readiness` for the structured report".to_owned(),
            );
            lines.push("  status: use /status for the current terminal UI summary".to_owned());
        }
        Some("sandbox") => {
            lines.push(
                "  sandbox: use /sandbox [--probe] for the human diagnostics view".to_owned(),
            );
        }
        Some("logs") => {
            lines.push("  logs: use `ikaros debug logs` for the structured log report".to_owned());
        }
        Some("insights") => {
            lines.push(
                "  insights: use `ikaros debug insights` for the structured report".to_owned(),
            );
        }
        Some("dump") => {
            lines.push(
                "  dump: use `ikaros debug dump` to write explicit debug artifacts".to_owned(),
            );
        }
        Some("state-db") | Some("state_db") => {
            lines.push(
                "  state db: use `ikaros debug state-db` for the structured report".to_owned(),
            );
        }
        Some("continuations") => {
            lines.push("  continuations: use /queue and /cancel for the human controls".to_owned());
        }
        Some(other) => {
            lines.push(format!("  filter: {}", terminal_inline(other)));
            lines.push(
                "  timeline: use /timeline, /replay, or /trace for session inspection".to_owned(),
            );
        }
        None => {
            lines.push(
                "  timeline: use /timeline, /replay, or /trace for session inspection".to_owned(),
            );
            lines.push(
                "  readiness: use `ikaros debug readiness` for structured diagnostics".to_owned(),
            );
        }
    }
    lines
}
