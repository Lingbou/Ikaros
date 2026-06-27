// SPDX-License-Identifier: GPL-3.0-only

mod open;

use crate::chat::attachments::content_block_kind;
use anyhow::Result;
use ikaros_agent::chat::ChatRunOptions;
use ikaros_core::IkarosPaths;
use ikaros_state::session::{SessionId, SessionStore, SqliteSessionStore};
use ikaros_terminal::terminal_inline;
use std::path::Path;

use super::continuations::{
    cancel_selected_screen_continuation, clear_selected_screen_input, continuations_json_line,
    handle_cancel_command,
};
use super::{InteractiveChatRuntime, InteractiveCommandContext, handle_approval_command};
use crate::chat::notice::{WorkbenchNotice, WorkbenchNoticeKind};
use crate::chat::workbench::{
    WorkbenchScreenApprovalAction, WorkbenchScreenContinuationAction, WorkbenchScreenInputAction,
    apply_workbench_screen_args, print_screen_status_with_state,
};
use open::handle_screen_open_selected_action;

pub(super) async fn handle_screen_command(
    args: Vec<&str>,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<()> {
    if !args.is_empty() {
        apply_workbench_screen_args(&mut runtime.screen_state, &args)?;
    }
    let mut handled_screen_action = false;
    if let Some(action) = runtime.screen_state.take_approval_action() {
        handle_screen_selected_approval_action(action, ctx.paths, ctx.workspace, runtime).await?;
        handled_screen_action = true;
    }
    if let Some(action) = runtime.screen_state.take_continuation_action() {
        handle_screen_selected_continuation_action(action, runtime)?;
        handled_screen_action = true;
    }
    if let Some(action) = runtime.screen_state.take_input_action() {
        handle_screen_selected_input_action(action, runtime)?;
        handled_screen_action = true;
    }
    if let Some(action) = runtime.screen_state.take_open_action() {
        handle_screen_open_selected_action(action, ctx, runtime, options).await?;
        handled_screen_action = true;
    }
    if runtime.fullscreen_stdout_quiet() {
        return Ok(());
    }
    if handled_screen_action && runtime.default_inline_stdout() && !runtime.screen_state.raw_mode()
    {
        return Ok(());
    }
    print_screen_status_with_state(
        ctx.config,
        ctx.paths,
        ctx.workspace,
        runtime,
        options,
        ctx.usage_ledger,
        &runtime.screen_state,
    )?;
    Ok(())
}

async fn handle_screen_selected_approval_action(
    action: WorkbenchScreenApprovalAction,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    let pending = runtime.session.pending_approvals()?;
    if pending.is_empty() {
        println!(
            "screen_approval_selected: action={} id=none reason=no_pending_approvals",
            screen_approval_action_name(action),
        );
        runtime.push_notice(WorkbenchNotice::new(
            WorkbenchNoticeKind::Error,
            "screen approval",
            "no pending approval is available",
        ));
        return Ok(());
    }
    let selected = runtime.screen_state.side_selection();
    let approval_rows = approval_side_panel_rows(pending.len());
    let selected_from_overlay = selected >= approval_rows;
    let pending_index = if selected_from_overlay {
        0
    } else {
        selected.checked_sub(1).unwrap_or(0)
    };
    let Some(record) = pending.get(pending_index) else {
        println!(
            "screen_approval_selected: action={} id=none reason=no_pending_approval_at_selection selected={}",
            screen_approval_action_name(action),
            selected.saturating_add(1)
        );
        runtime.push_notice(WorkbenchNotice::new(
            WorkbenchNoticeKind::Error,
            "screen approval",
            "no pending approval at selected side-panel row",
        ));
        return Ok(());
    };
    let approval_id = record.request.id.clone();
    println!(
        "screen_approval_selected: action={} id={} source={}",
        screen_approval_action_name(action),
        terminal_inline(&approval_id),
        if selected_from_overlay {
            "approval_overlay"
        } else {
            "side_selection"
        }
    );
    match action {
        WorkbenchScreenApprovalAction::Approve => {
            handle_approval_command(
                vec!["approve", approval_id.as_str()],
                paths,
                workspace,
                runtime,
                "screen_selected",
            )
            .await?;
            runtime.request_pending_input_drain();
        }
        WorkbenchScreenApprovalAction::Deny => {
            handle_approval_command(
                vec!["deny", approval_id.as_str()],
                paths,
                workspace,
                runtime,
                "screen_selected",
            )
            .await?;
        }
    }
    runtime.push_notice(WorkbenchNotice::new(
        WorkbenchNoticeKind::Approval,
        "screen approval",
        &format!(
            "action={} approval_id={} next=/screen timeline=/timeline trace=/trace",
            screen_approval_action_name(action),
            terminal_inline(&approval_id)
        ),
    ));
    Ok(())
}

fn screen_approval_action_name(action: WorkbenchScreenApprovalAction) -> &'static str {
    match action {
        WorkbenchScreenApprovalAction::Approve => "approve",
        WorkbenchScreenApprovalAction::Deny => "deny",
    }
}

fn handle_screen_selected_continuation_action(
    action: WorkbenchScreenContinuationAction,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let pending_approval_rows =
        approval_side_panel_rows(runtime.session.pending_approvals()?.len());
    let selected = runtime.screen_state.side_selection();
    match action {
        WorkbenchScreenContinuationAction::Cancel => {
            let selected_report = cancel_selected_screen_continuation(
                &store,
                &session_id,
                pending_approval_rows,
                &runtime.pending_inputs,
                selected,
                "workbench screen selected cancel",
            )?;
            let Some(continuation_id) = selected_report.continuation_id else {
                if screen_has_active_cancel_target(runtime) {
                    handle_cancel_command(vec!["all"], runtime)?;
                    println!(
                        "screen_continuation_selected: action=cancel id=all source=active_progress selected={}",
                        selected.saturating_add(1)
                    );
                    runtime.push_notice(WorkbenchNotice::new(
                        WorkbenchNoticeKind::Continuation,
                        "screen cancel",
                        "no selected continuation; cancelled active turn/queue instead",
                    ));
                    return Ok(());
                }
                println!(
                    "screen_continuation_selected: action=cancel id=none reason=no_continuation_at_selection selected={}",
                    selected.saturating_add(1)
                );
                runtime.push_notice(WorkbenchNotice::new(
                    WorkbenchNoticeKind::Error,
                    "screen continuation",
                    "no continuation at selected side-panel row",
                ));
                return Ok(());
            };
            println!(
                "screen_continuation_selected: action=cancel id={}",
                terminal_inline(&continuation_id)
            );
            println!(
                "workbench_cancel: target={} cancelled={} skipped={} missing={}",
                terminal_inline(&continuation_id),
                selected_report.report.cancelled,
                selected_report.report.skipped,
                selected_report.report.missing
            );
            println!(
                "{}",
                continuations_json_line(&store.continuations(&session_id)?)
            );
            runtime.push_notice(WorkbenchNotice::new(
                WorkbenchNoticeKind::Continuation,
                "screen continuation",
                &format!(
                    "action=cancel continuation_id={} cancelled={} skipped={} missing={} next=/screen /debug continuations",
                    terminal_inline(&continuation_id),
                    selected_report.report.cancelled,
                    selected_report.report.skipped,
                    selected_report.report.missing
                ),
            ));
        }
    }
    Ok(())
}

fn screen_has_active_cancel_target(runtime: &InteractiveChatRuntime) -> bool {
    runtime.last_progress.as_ref().is_some_and(|progress| {
        matches!(
            progress.status.as_str(),
            "running" | "queued" | "approval_pending" | "failed"
        )
    }) || !runtime.pending_inputs.is_empty()
}

fn handle_screen_selected_input_action(
    action: WorkbenchScreenInputAction,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    let pending_approval_rows =
        approval_side_panel_rows(runtime.session.pending_approvals()?.len());
    let selected = runtime.screen_state.side_selection();
    match action {
        WorkbenchScreenInputAction::Clear => {
            let attachment_offset = pending_approval_rows + runtime.pending_inputs.len().min(4);
            if selected >= attachment_offset
                && selected < attachment_offset + runtime.pending_content_blocks.len().min(4)
            {
                let attachment_index = selected - attachment_offset;
                let removed = runtime.pending_content_blocks.remove(attachment_index);
                println!(
                    "screen_attachment_selected: action=clear index={} kind={}",
                    attachment_index + 1,
                    content_block_kind(&removed)
                );
                println!(
                    "attachment_removed: index={} remaining={}",
                    attachment_index + 1,
                    runtime.pending_content_blocks.len()
                );
                runtime.push_notice(WorkbenchNotice::new(
                    WorkbenchNoticeKind::Context,
                    "screen attachment",
                    &format!(
                        "action=clear index={} remaining={} next=/attach /screen",
                        attachment_index + 1,
                        runtime.pending_content_blocks.len()
                    ),
                ));
                return Ok(());
            }
            let report = clear_selected_screen_input(
                pending_approval_rows,
                &mut runtime.pending_inputs,
                selected,
            );
            let Some(input_index) = report.input_index else {
                println!(
                    "screen_input_selected: action=clear index=none reason=no_pending_input_at_selection selected={}",
                    selected.saturating_add(1)
                );
                runtime.push_notice(WorkbenchNotice::new(
                    WorkbenchNoticeKind::Error,
                    "screen input",
                    "no queued input at selected side-panel row",
                ));
                return Ok(());
            };
            println!(
                "screen_input_selected: action=clear index={} message={}",
                input_index,
                report
                    .removed
                    .as_deref()
                    .map(terminal_inline)
                    .unwrap_or_else(|| "none".into())
            );
            println!(
                "pending_input_removed: index={} remaining={}",
                input_index, report.remaining
            );
            runtime.push_notice(WorkbenchNotice::new(
                WorkbenchNoticeKind::Continuation,
                "screen input",
                &format!(
                    "action=clear input_index={} remaining={} next=/queue run /queue clear",
                    input_index, report.remaining
                ),
            ));
        }
    }
    Ok(())
}

fn approval_side_panel_rows(pending_approvals: usize) -> usize {
    if pending_approvals == 0 {
        0
    } else {
        1 + pending_approvals.min(4)
    }
}
