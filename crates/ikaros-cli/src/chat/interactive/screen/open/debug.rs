// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::parse::{parse_debug_dump_recent_logs, parse_debug_logs_args};
use crate::chat::interactive::screen::open::command_parse::parse_debug_memory_lifecycle_args;
use crate::chat::interactive::screen::open::outcome::ScreenOpenCommandStatus;
use crate::chat::interactive::{
    InteractiveChatRuntime, InteractiveCommandContext, print_debug_status_for_human,
};
use crate::chat::workbench::{
    TimelineRequest, TimelineVerbosity, print_replay_status, print_replay_status_for_human,
};
use crate::debug::{
    debug_dump_json_line, debug_insights_json_line, debug_logs_json_line,
    debug_memory_lifecycle_json_line, debug_readiness_json_line, debug_sandbox_json_line,
    debug_state_db_json_line,
};
use anyhow::Result;

pub(super) async fn execute_screen_open_debug_command(
    subcommand: Option<&str>,
    args: &[&str],
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    use_human_output: bool,
) -> Result<ScreenOpenCommandStatus> {
    match subcommand {
        Some("readiness") => {
            if use_human_output {
                print_debug_status_for_human(&["readiness"]);
            } else {
                println!("readiness: see readiness_json for structured MVP status");
                println!(
                    "{}",
                    debug_readiness_json_line(ctx.paths, ctx.workspace, Some(&runtime.agent.name))?
                );
            }
        }
        Some("sandbox") => {
            if args.first().copied() == Some("--probe") {
                return Ok(ScreenOpenCommandStatus::ExplicitActionRequired);
            }
            if use_human_output {
                print_debug_status_for_human(&["sandbox"]);
            } else {
                println!(
                    "{}",
                    debug_sandbox_json_line(
                        ctx.paths,
                        ctx.workspace,
                        Some(&runtime.agent.name),
                        false
                    )
                    .await?
                );
            }
        }
        Some("logs") => {
            let (source, page, page_size) = parse_debug_logs_args(args)?;
            println!(
                "{}",
                debug_logs_json_line(ctx.paths, source, page, page_size)?
            );
        }
        Some("insights") => {
            println!(
                "{}",
                debug_insights_json_line(ctx.paths, ctx.workspace, Some(&runtime.agent.name))?
            );
        }
        Some("dump") => {
            let recent_logs = parse_debug_dump_recent_logs(args)?;
            println!(
                "{}",
                debug_dump_json_line(
                    ctx.paths,
                    ctx.workspace,
                    Some(&runtime.agent.name),
                    recent_logs
                )?
            );
        }
        Some("state-db" | "state_db") => {
            println!(
                "{}",
                debug_state_db_json_line(ctx.paths, ctx.workspace, Some(&runtime.agent.name))?
            );
        }
        Some("continuations") => {
            if use_human_output {
                print_debug_status_for_human(&["continuations"]);
            } else {
                super::super::super::continuations::print_workbench_continuation_status(runtime)?;
            }
        }
        Some("memory-lifecycle" | "memory_lifecycle") => {
            let (session_id, turn_id) =
                parse_debug_memory_lifecycle_args(args, &runtime.chat_session_id);
            println!(
                "{}",
                debug_memory_lifecycle_json_line(
                    ctx.paths,
                    ctx.workspace,
                    Some(&runtime.agent.name),
                    &session_id,
                    turn_id.as_deref(),
                )?
            );
        }
        Some(turn_id) => {
            let request = TimelineRequest::for_turn(turn_id);
            if use_human_output {
                print_replay_status_for_human(
                    "debug",
                    ctx.config,
                    ctx.paths,
                    ctx.workspace,
                    runtime,
                    TimelineVerbosity::Debug,
                    request,
                )?;
            } else {
                print_replay_status(
                    "debug",
                    ctx.config,
                    ctx.paths,
                    ctx.workspace,
                    runtime,
                    TimelineVerbosity::Debug,
                    request,
                )?;
            }
        }
        None => {
            return Ok(ScreenOpenCommandStatus::Unsupported);
        }
    }
    Ok(ScreenOpenCommandStatus::Executed)
}
