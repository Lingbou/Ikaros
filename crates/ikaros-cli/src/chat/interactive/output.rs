// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_terminal::{print_inline_history_lines, terminal_inline};

use crate::chat::notice::WorkbenchNotice;
use crate::chat::slash::{SlashCommandFullscreenEffect, slash_command_fullscreen_effect};
use crate::chat::workbench::apply_workbench_screen_args;

use super::InteractiveChatRuntime;

pub(in crate::chat::interactive) fn print_default_inline_lines(lines: Vec<String>) -> Result<()> {
    print_inline_history_lines(&lines)
}

pub(in crate::chat) fn suppress_fullscreen_stdout_command(
    input: &str,
    runtime: &mut InteractiveChatRuntime,
) -> Result<bool> {
    if !runtime.fullscreen_stdout_quiet() {
        return Ok(false);
    }
    let command = input.split_whitespace().next().unwrap_or_default();
    match slash_command_fullscreen_effect(input) {
        SlashCommandFullscreenEffect::Inspect => {
            if matches!(command, "/help" | "/commands") {
                apply_workbench_screen_args(&mut runtime.screen_state, &["--palette"])?;
                runtime.push_notice(WorkbenchNotice::info(
                    "command palette",
                    "opened slash command picker",
                ));
            } else {
                runtime.push_notice(WorkbenchNotice::info(
                    "command routed",
                    &format!(
                        "{} is shown through the fullscreen workbench instead of raw terminal output",
                        terminal_inline(command)
                    ),
                ));
            }
            Ok(true)
        }
        SlashCommandFullscreenEffect::ActionOrProbe => {
            runtime.push_notice(WorkbenchNotice::info(
                "command running",
                &format!(
                    "{} will run and refresh the fullscreen workbench",
                    terminal_inline(input)
                ),
            ));
            Ok(false)
        }
        SlashCommandFullscreenEffect::TerminalStdout => Ok(false),
    }
}
