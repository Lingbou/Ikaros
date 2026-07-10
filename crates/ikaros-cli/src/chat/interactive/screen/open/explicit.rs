// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::screen::open::command_parse::{
    command_tail, screen_budget_command_resumes_pending_inputs,
};
use crate::chat::interactive::screen::open::outcome::ScreenOpenCommandStatus;
use crate::chat::interactive::{
    InteractiveChatRuntime, InteractiveCommandContext,
    provider::{handle_budget_command, handle_provider_command},
};
use anyhow::Result;
use ikaros_terminal::terminal_inline;

pub(super) async fn execute_confirmed_explicit_command(
    command: &str,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<ScreenOpenCommandStatus> {
    let mut parts = command.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("/code"), _) => {
            super::code::execute_screen_code_command(command, ctx, runtime, true).await
        }
        (Some("/rollback"), _) => {
            let code_command = format!("/code rollback {}", command_tail(command));
            super::code::execute_screen_code_command(&code_command, ctx, runtime, true).await
        }
        (Some("/budget"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            handle_budget_command(args, ctx.paths, runtime)?;
            if screen_budget_command_resumes_pending_inputs(command) {
                runtime.request_pending_input_drain();
            }
            Ok(ScreenOpenCommandStatus::Executed)
        }
        (Some("/provider"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            handle_provider_command(args, ctx.paths, ctx.workspace, runtime).await?;
            Ok(ScreenOpenCommandStatus::Executed)
        }
        _ => {
            println!(
                "screen_confirm_selected: unsupported_explicit_command command={}",
                terminal_inline(&command)
            );
            Ok(ScreenOpenCommandStatus::ExplicitActionRequired)
        }
    }
}
