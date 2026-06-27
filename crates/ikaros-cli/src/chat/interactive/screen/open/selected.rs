// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_agent::chat::ChatRunOptions;
use ikaros_terminal::terminal_inline;

use crate::chat::notice::{WorkbenchNotice, WorkbenchNoticeKind};
use crate::chat::workbench::{WorkbenchScreenOpenAction, selected_screen_primary_action};

use super::super::super::{InteractiveChatRuntime, InteractiveCommandContext};
use super::outcome::{print_screen_open_selected_status, screen_open_notice_kind};

pub(in crate::chat::interactive::screen) async fn handle_screen_open_selected_action(
    action: WorkbenchScreenOpenAction,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<()> {
    match action {
        WorkbenchScreenOpenAction::OpenSelected | WorkbenchScreenOpenAction::ConfirmSelected => {
            let quiet_machine_output = runtime.fullscreen_stdout_quiet()
                || (runtime.default_inline_stdout() && !runtime.screen_state.raw_mode());
            let Some(command) = selected_screen_primary_action(
                ctx.config,
                ctx.paths,
                ctx.workspace,
                runtime,
                options,
                ctx.usage_ledger,
                &runtime.screen_state,
            )?
            else {
                if !quiet_machine_output {
                    println!("screen_open_selected: command=none");
                    println!("screen_open_selected_status: not_found");
                }
                runtime.push_notice(WorkbenchNotice::new(
                    WorkbenchNoticeKind::Error,
                    "screen open selected",
                    "no action command for selected workbench cell",
                ));
                return Ok(());
            };
            if !quiet_machine_output {
                println!(
                    "screen_open_selected: command={} confirmed={}",
                    terminal_inline(&command),
                    action == WorkbenchScreenOpenAction::ConfirmSelected,
                );
            }
            let accepted_command_palette = runtime.screen_state.command_palette_open();
            if accepted_command_palette {
                runtime.screen_state.close_command_palette();
            }
            let status = super::execute_screen_open_command(
                &command,
                ctx,
                runtime,
                options,
                action == WorkbenchScreenOpenAction::ConfirmSelected,
            )
            .await?;
            if !quiet_machine_output {
                print_screen_open_selected_status(status, &command);
            }
            runtime.push_notice(WorkbenchNotice::new(
                screen_open_notice_kind(status),
                "screen open selected",
                &format!(
                    "status={} confirmed={} command={} next=/screen /timeline /trace",
                    status.as_str(),
                    action == WorkbenchScreenOpenAction::ConfirmSelected,
                    terminal_inline(&command)
                ),
            ));
        }
    }
    Ok(())
}
