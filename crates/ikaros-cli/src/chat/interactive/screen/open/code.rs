// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result};
use ikaros_terminal::terminal_inline;

use crate::chat::notice::{WorkbenchNotice, WorkbenchNoticeKind};
use crate::code::{code_command, parse_interactive_code_command};

use super::super::super::{InteractiveChatRuntime, InteractiveCommandContext};
use super::command_parse::{command_tail, normalize_screen_code_command};
use super::outcome::ScreenOpenCommandStatus;

pub(super) async fn execute_screen_code_command(
    command: &str,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    allow_explicit: bool,
) -> Result<ScreenOpenCommandStatus> {
    let command_line = match normalize_screen_code_command(command) {
        Some(command_line) => command_line,
        None if allow_explicit => command_tail(command),
        None => return Ok(ScreenOpenCommandStatus::ExplicitActionRequired),
    };
    let parsed = parse_interactive_code_command(&command_line)
        .with_context(|| format!("failed to parse selected {command}"))?;
    code_command(parsed, ctx.paths, ctx.workspace, Some(&runtime.agent.name)).await?;
    runtime.push_notice(WorkbenchNotice::new(
        WorkbenchNoticeKind::Progress,
        "screen code",
        &format!(
            "status=executed command={} timeline=/timeline --kind coding trace=/trace --kind coding",
            terminal_inline(&format!("/code {command_line}"))
        ),
    ));
    Ok(ScreenOpenCommandStatus::Executed)
}
