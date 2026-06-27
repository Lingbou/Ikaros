// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;

use crate::chat::workbench::{
    TimelineVerbosity, print_replay_status, print_replay_status_for_human, print_trace_status,
    print_trace_status_for_human,
};

use super::super::super::parse::{parse_timeline_request, parse_trace_request};
use super::super::super::{InteractiveChatRuntime, InteractiveCommandContext};
use super::outcome::ScreenOpenCommandStatus;

pub(super) fn execute_replay_status_command(
    label: &str,
    verbosity: TimelineVerbosity,
    command: &str,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &InteractiveChatRuntime,
    use_human_output: bool,
) -> Result<ScreenOpenCommandStatus> {
    let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
    let request = parse_timeline_request(args)?;
    if use_human_output {
        print_replay_status_for_human(
            label,
            ctx.config,
            ctx.paths,
            ctx.workspace,
            runtime,
            verbosity,
            request,
        )?;
    } else {
        print_replay_status(
            label,
            ctx.config,
            ctx.paths,
            ctx.workspace,
            runtime,
            verbosity,
            request,
        )?;
    }
    Ok(ScreenOpenCommandStatus::Executed)
}

pub(super) fn execute_trace_status_command(
    command: &str,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &InteractiveChatRuntime,
    use_human_output: bool,
) -> Result<ScreenOpenCommandStatus> {
    let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
    let request = parse_trace_request(args)?;
    if use_human_output {
        print_trace_status_for_human(ctx.config, ctx.paths, ctx.workspace, runtime, request)?;
    } else {
        print_trace_status(ctx.config, ctx.paths, ctx.workspace, runtime, request)?;
    }
    Ok(ScreenOpenCommandStatus::Executed)
}
