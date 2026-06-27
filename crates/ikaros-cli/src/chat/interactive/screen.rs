// SPDX-License-Identifier: GPL-3.0-only

use crate::browser::run_browser_workbench_command;
use crate::chat::attachments::content_block_kind;
use crate::code::{code_command, parse_interactive_code_command};
use crate::debug::{
    debug_dump_json_line, debug_insights_json_line, debug_logs_json_line,
    debug_memory_lifecycle_json_line, debug_readiness_json_line, debug_sandbox_json_line,
    debug_state_db_json_line,
};
use crate::message::{run_gateway_adapter_workbench_command, run_gateway_daemon_workbench_command};
use anyhow::{Context, Result};
use ikaros_core::IkarosPaths;
use ikaros_runtime::ChatRunOptions;
use ikaros_session::{SessionId, SessionStore, SqliteSessionStore};
use std::path::Path;

use super::attachment::handle_attach_command;
use super::continuations::{
    cancel_selected_screen_continuation, clear_selected_screen_input, continuations_json_line,
    handle_cancel_command, print_workbench_continuation_status,
};
use super::evidence::append_workbench_evidence;
use super::parse::{parse_timeline_request, parse_trace_request};
use super::provider::handle_budget_command;
use super::{
    InteractiveChatRuntime, InteractiveCommandContext, handle_approval_command,
    handle_image_command, handle_provider_command, handle_vision_command, handle_web_command,
    parse_debug_dump_recent_logs, parse_debug_logs_args, print_image_usage, print_vision_usage,
    print_web_usage, terminal_inline,
};
use crate::chat::notice::{WorkbenchNotice, WorkbenchNoticeKind};
use crate::chat::workbench::{
    TimelineRequest, TimelineVerbosity, WorkbenchScreenApprovalAction,
    WorkbenchScreenContinuationAction, WorkbenchScreenInputAction, WorkbenchScreenOpenAction,
    apply_workbench_screen_args, command_requires_explicit_action, format_workbench_help,
    print_context_status, print_diff_status, print_gateway_status, print_mcp_status,
    print_memory_status, print_model_status, print_persistent_screen_status_with_state,
    print_rag_status, print_replay_status, print_screen_status_with_state, print_slash_commands,
    print_tools_status, print_trace_status, print_workbench_status, selected_screen_primary_action,
};

pub(super) async fn handle_screen_command(
    args: Vec<&str>,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<()> {
    if !args.is_empty() {
        apply_workbench_screen_args(&mut runtime.screen_state, &args)?;
    }
    if let Some(action) = runtime.screen_state.take_approval_action() {
        handle_screen_selected_approval_action(action, ctx.paths, ctx.workspace, runtime).await?;
    }
    if let Some(action) = runtime.screen_state.take_continuation_action() {
        handle_screen_selected_continuation_action(action, runtime)?;
    }
    if let Some(action) = runtime.screen_state.take_input_action() {
        handle_screen_selected_input_action(action, runtime)?;
    }
    if let Some(action) = runtime.screen_state.take_open_action() {
        handle_screen_open_selected_action(action, ctx, runtime, options).await?;
    }
    if runtime.persistent_fullscreen && runtime.screen_state.fullscreen() {
        print_persistent_screen_status_with_state(
            ctx.config,
            ctx.paths,
            ctx.workspace,
            runtime,
            options,
            ctx.usage_ledger,
            &runtime.screen_state,
        )?;
    } else {
        print_screen_status_with_state(
            ctx.config,
            ctx.paths,
            ctx.workspace,
            runtime,
            options,
            ctx.usage_ledger,
            &runtime.screen_state,
        )?;
    }
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

async fn handle_screen_open_selected_action(
    action: WorkbenchScreenOpenAction,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
) -> Result<()> {
    match action {
        WorkbenchScreenOpenAction::OpenSelected | WorkbenchScreenOpenAction::ConfirmSelected => {
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
                if !runtime.fullscreen_stdout_quiet() {
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
            if !runtime.fullscreen_stdout_quiet() {
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
            let status = execute_screen_open_command(
                &command,
                ctx,
                runtime,
                options,
                action == WorkbenchScreenOpenAction::ConfirmSelected,
            )
            .await?;
            if !runtime.fullscreen_stdout_quiet() {
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

async fn execute_confirmed_explicit_command(
    command: &str,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<ScreenOpenCommandStatus> {
    let mut parts = command.split_whitespace();
    match (parts.next(), parts.next()) {
        (Some("/code"), _) => execute_screen_code_command(command, ctx, runtime, true).await,
        (Some("/rollback"), _) => {
            let code_command = format!("/code rollback {}", command_tail(command));
            execute_screen_code_command(&code_command, ctx, runtime, true).await
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
        (Some("/browser"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            run_browser_workbench_command(&runtime.session, ctx.paths, &args).await?;
            append_workbench_evidence(runtime, "browser", serde_json::json!({"args": args}))?;
            Ok(ScreenOpenCommandStatus::Executed)
        }
        (Some("/web"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            handle_web_command(args.clone(), ctx, runtime).await?;
            append_workbench_evidence(runtime, "web", serde_json::json!({"args": args}))?;
            Ok(ScreenOpenCommandStatus::Executed)
        }
        (Some("/vision"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            handle_vision_command(args.clone(), ctx, runtime).await?;
            append_workbench_evidence(runtime, "vision", serde_json::json!({"args": args}))?;
            Ok(ScreenOpenCommandStatus::Executed)
        }
        (Some("/image"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            handle_image_command(args.clone(), ctx, runtime).await?;
            append_workbench_evidence(runtime, "image", serde_json::json!({"args": args}))?;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScreenOpenCommandStatus {
    Executed,
    ExplicitActionRequired,
    Unsupported,
}

impl ScreenOpenCommandStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Executed => "executed",
            Self::ExplicitActionRequired => "explicit_action_required",
            Self::Unsupported => "unsupported",
        }
    }
}

fn screen_open_notice_kind(status: ScreenOpenCommandStatus) -> WorkbenchNoticeKind {
    match status {
        ScreenOpenCommandStatus::Executed => WorkbenchNoticeKind::Info,
        ScreenOpenCommandStatus::ExplicitActionRequired => WorkbenchNoticeKind::Progress,
        ScreenOpenCommandStatus::Unsupported => WorkbenchNoticeKind::Error,
    }
}

fn print_screen_open_selected_status(status: ScreenOpenCommandStatus, command: &str) {
    match status {
        ScreenOpenCommandStatus::Executed => println!("screen_open_selected_status: executed"),
        ScreenOpenCommandStatus::ExplicitActionRequired => println!(
            "screen_open_selected_status: explicit_action_required command={}",
            terminal_inline(command)
        ),
        ScreenOpenCommandStatus::Unsupported => println!(
            "screen_open_selected_status: unsupported command={}",
            terminal_inline(command)
        ),
    }
}

async fn execute_screen_open_command(
    command: &str,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    allow_explicit: bool,
) -> Result<ScreenOpenCommandStatus> {
    if command_requires_explicit_action(command) && !allow_explicit {
        return Ok(ScreenOpenCommandStatus::ExplicitActionRequired);
    }
    if super::suppress_fullscreen_stdout_command(command, runtime)? {
        return Ok(ScreenOpenCommandStatus::Executed);
    }
    let mut parts = command.split_whitespace();
    let status = match (parts.next(), parts.next()) {
        (Some("/timeline"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            print_replay_status(
                "timeline",
                ctx.config,
                ctx.paths,
                ctx.workspace,
                runtime,
                TimelineVerbosity::Timeline,
                parse_timeline_request(args)?,
            )?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/replay"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            print_replay_status(
                "replay",
                ctx.config,
                ctx.paths,
                ctx.workspace,
                runtime,
                TimelineVerbosity::Replay,
                parse_timeline_request(args)?,
            )?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/trace"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            print_trace_status(
                ctx.config,
                ctx.paths,
                ctx.workspace,
                runtime,
                parse_trace_request(args)?,
            )?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/debug"), Some("readiness")) => {
            println!("readiness: see readiness_json for structured MVP status");
            println!(
                "{}",
                debug_readiness_json_line(ctx.paths, ctx.workspace, Some(&runtime.agent.name))?
            );
            ScreenOpenCommandStatus::Executed
        }
        (Some("/debug"), Some("sandbox")) => {
            if parts.next() == Some("--probe") {
                ScreenOpenCommandStatus::ExplicitActionRequired
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
                ScreenOpenCommandStatus::Executed
            }
        }
        (Some("/debug"), Some("logs")) => {
            let args = parts.collect::<Vec<_>>();
            let (source, page, page_size) = parse_debug_logs_args(&args)?;
            println!(
                "{}",
                debug_logs_json_line(ctx.paths, source, page, page_size)?
            );
            ScreenOpenCommandStatus::Executed
        }
        (Some("/debug"), Some("insights")) => {
            println!(
                "{}",
                debug_insights_json_line(ctx.paths, ctx.workspace, Some(&runtime.agent.name))?
            );
            ScreenOpenCommandStatus::Executed
        }
        (Some("/debug"), Some("dump")) => {
            let args = parts.collect::<Vec<_>>();
            let recent_logs = parse_debug_dump_recent_logs(&args)?;
            println!(
                "{}",
                debug_dump_json_line(
                    ctx.paths,
                    ctx.workspace,
                    Some(&runtime.agent.name),
                    recent_logs
                )?
            );
            ScreenOpenCommandStatus::Executed
        }
        (Some("/debug"), Some("state-db" | "state_db")) => {
            println!(
                "{}",
                debug_state_db_json_line(ctx.paths, ctx.workspace, Some(&runtime.agent.name))?
            );
            ScreenOpenCommandStatus::Executed
        }
        (Some("/debug"), Some("continuations")) => {
            print_workbench_continuation_status(runtime)?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/debug"), Some("memory-lifecycle" | "memory_lifecycle")) => {
            let args = parts.collect::<Vec<_>>();
            let (session_id, turn_id) =
                parse_debug_memory_lifecycle_args(&args, &runtime.chat_session_id);
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
            ScreenOpenCommandStatus::Executed
        }
        (Some("/debug"), Some(turn_id)) => {
            print_replay_status(
                "debug",
                ctx.config,
                ctx.paths,
                ctx.workspace,
                runtime,
                TimelineVerbosity::Debug,
                TimelineRequest::for_turn(turn_id),
            )?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/context"), _) => {
            print_context_status(runtime, options)?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/memory"), _) => {
            print_memory_status(ctx.config, ctx.paths, runtime)?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/rag"), _) => {
            print_rag_status(ctx.config, ctx.paths, options);
            ScreenOpenCommandStatus::Executed
        }
        (Some("/diff"), _) => {
            print_diff_status(runtime, ctx.workspace).await?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/code"), _) => {
            execute_screen_code_command(command, ctx, runtime, allow_explicit).await?
        }
        (Some("/review"), _) => {
            let code_command = format!("/code review {}", command_tail(command));
            execute_screen_code_command(&code_command, ctx, runtime, allow_explicit).await?
        }
        (Some("/rollback"), _) => {
            if allow_explicit {
                execute_confirmed_explicit_command(command, ctx, runtime).await?
            } else {
                ScreenOpenCommandStatus::ExplicitActionRequired
            }
        }
        (Some("/model"), _) => {
            if runtime.fullscreen_stdout_quiet() {
                runtime.push_notice(WorkbenchNotice::info(
                    "model",
                    "model status refreshed in the workbench",
                ));
            } else {
                print_model_status(ctx.paths, runtime)?;
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/status"), _) => {
            print_workbench_status(
                ctx.config,
                ctx.paths,
                ctx.workspace,
                runtime,
                options,
                ctx.usage_ledger,
            )?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/help"), _) => {
            println!("{}", format_workbench_help());
            ScreenOpenCommandStatus::Executed
        }
        (Some("/budget"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            handle_budget_command(args, ctx.paths, runtime)?;
            if screen_budget_command_resumes_pending_inputs(command) {
                runtime.request_pending_input_drain();
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/attach"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            handle_attach_command(args, runtime, ctx.workspace)?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/tools"), _) => {
            print_tools_status(ctx.registry, &runtime.agent)?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/commands"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            print_slash_commands(&args);
            ScreenOpenCommandStatus::Executed
        }
        (Some("/mcp"), None) | (Some("/mcp"), Some("status")) => {
            print_mcp_status(ctx.config);
            ScreenOpenCommandStatus::Executed
        }
        (Some("/api"), None) | (Some("/api"), Some("status")) => {
            super::print_api_status(ctx.config);
            ScreenOpenCommandStatus::Executed
        }
        (Some("/browser"), None)
        | (Some("/browser"), Some("status" | "list" | "supervisor-status" | "supervisor")) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            run_browser_workbench_command(&runtime.session, ctx.paths, &args).await?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/web"), None) | (Some("/web"), Some("help" | "--help")) => {
            print_web_usage();
            ScreenOpenCommandStatus::Executed
        }
        (Some("/web"), Some("search" | "extract")) => {
            if allow_explicit {
                execute_confirmed_explicit_command(command, ctx, runtime).await?
            } else {
                ScreenOpenCommandStatus::ExplicitActionRequired
            }
        }
        (Some("/vision"), None) | (Some("/vision"), Some("help" | "--help")) => {
            print_vision_usage();
            ScreenOpenCommandStatus::Executed
        }
        (Some("/vision"), Some("describe")) if parts.next().is_none() => {
            print_vision_usage();
            ScreenOpenCommandStatus::Executed
        }
        (Some("/image"), None) | (Some("/image"), Some("help" | "--help")) => {
            print_image_usage();
            ScreenOpenCommandStatus::Executed
        }
        (Some("/image"), Some("generate")) if parts.next().is_none() => {
            print_image_usage();
            ScreenOpenCommandStatus::Executed
        }
        (Some("/browser"), _) | (Some("/web"), _) | (Some("/vision"), _) | (Some("/image"), _) => {
            if allow_explicit {
                execute_confirmed_explicit_command(command, ctx, runtime).await?
            } else {
                ScreenOpenCommandStatus::ExplicitActionRequired
            }
        }
        (Some("/provider"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            if args.contains(&"--live") && !allow_explicit {
                ScreenOpenCommandStatus::ExplicitActionRequired
            } else {
                handle_provider_command(args, ctx.paths, ctx.workspace, runtime).await?;
                ScreenOpenCommandStatus::Executed
            }
        }
        (Some("/gateway"), Some("daemon")) => {
            let args = command.split_whitespace().skip(2).collect::<Vec<_>>();
            if matches!(args.as_slice(), [] | ["status"]) {
                run_gateway_daemon_workbench_command(
                    &args,
                    ctx.paths,
                    ctx.workspace,
                    Some(&runtime.agent.name),
                )?;
                ScreenOpenCommandStatus::Executed
            } else {
                ScreenOpenCommandStatus::ExplicitActionRequired
            }
        }
        (Some("/gateway"), Some("adapter")) => {
            let args = command.split_whitespace().skip(2).collect::<Vec<_>>();
            if matches!(args.as_slice(), [] | ["list"] | ["status"]) {
                run_gateway_adapter_workbench_command(&args, ctx.paths)?;
                ScreenOpenCommandStatus::Executed
            } else {
                ScreenOpenCommandStatus::ExplicitActionRequired
            }
        }
        (Some("/gateway"), _) => {
            print_gateway_status(ctx.paths)?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/screen"), Some("approve-selected" | "approve")) => {
            handle_screen_selected_approval_action(
                WorkbenchScreenApprovalAction::Approve,
                ctx.paths,
                ctx.workspace,
                runtime,
            )
            .await?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/screen"), Some("deny-selected" | "deny")) => {
            handle_screen_selected_approval_action(
                WorkbenchScreenApprovalAction::Deny,
                ctx.paths,
                ctx.workspace,
                runtime,
            )
            .await?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/screen"), Some("cancel-selected" | "cancel")) => {
            handle_screen_selected_continuation_action(
                WorkbenchScreenContinuationAction::Cancel,
                runtime,
            )?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/screen"), Some("clear-selected" | "clear")) => {
            handle_screen_selected_input_action(WorkbenchScreenInputAction::Clear, runtime)?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/approval"), Some("approve" | "deny")) => {
            let action = command
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            handle_approval_command(
                command.split_whitespace().skip(1).collect::<Vec<_>>(),
                ctx.paths,
                ctx.workspace,
                runtime,
                "screen_open_selected",
            )
            .await?;
            if action == "approve" {
                runtime.request_pending_input_drain();
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/cancel"), _) => {
            handle_cancel_command(
                command.split_whitespace().skip(1).collect::<Vec<_>>(),
                runtime,
            )?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/queue"), Some("run" | "drain" | "continue")) => {
            runtime.request_pending_input_drain();
            if !runtime.fullscreen_stdout_quiet() {
                println!(
                    "screen_queue_run_requested: pending_inputs={}",
                    runtime.pending_inputs.len()
                );
            }
            runtime.push_notice(WorkbenchNotice::new(
                WorkbenchNoticeKind::Continuation,
                "screen queue",
                "drain requested from selected workbench action",
            ));
            ScreenOpenCommandStatus::Executed
        }
        (Some("/queue"), Some("retry" | "requeue")) => {
            super::continuations::handle_queue_command(
                command.split_whitespace().skip(1).collect::<Vec<_>>(),
                runtime,
            )?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/approval"), _) | (Some("/queue"), _) => {
            ScreenOpenCommandStatus::ExplicitActionRequired
        }
        _ => ScreenOpenCommandStatus::Unsupported,
    };
    Ok(status)
}

async fn execute_screen_code_command(
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

fn normalize_screen_code_command(command: &str) -> Option<String> {
    let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
    let subcommand = args.first().copied()?;
    match subcommand {
        "apply" | "rollback" | "guarded-edit" | "guarded_edit" => None,
        "plan" => Some(normalize_code_command_with_default_objective(
            &args,
            "inspect the current workspace and propose the next coding plan",
        )),
        "workflow" => {
            if args.iter().any(|arg| *arg == "--apply-patch") {
                None
            } else {
                Some(normalize_code_command_with_default_objective(
                    &args,
                    "continue the current coding workflow",
                ))
            }
        }
        "test" | "review" | "iterate" => Some(args.join(" ")),
        _ => None,
    }
}

fn command_tail(command: &str) -> String {
    command
        .split_whitespace()
        .skip(1)
        .collect::<Vec<_>>()
        .join(" ")
}

fn screen_budget_command_resumes_pending_inputs(command: &str) -> bool {
    let mut parts = command.split_whitespace();
    matches!(parts.next(), Some("/budget"))
        && matches!(parts.next(), Some("set" | "disable" | "off"))
}

fn normalize_code_command_with_default_objective(args: &[&str], default_objective: &str) -> String {
    if code_args_have_positional_objective(args) {
        return args.join(" ");
    }
    let mut normalized = vec![args[0].to_owned(), shell_quote(default_objective)];
    normalized.extend(args.iter().skip(1).map(|arg| (*arg).to_owned()));
    normalized.join(" ")
}

fn code_args_have_positional_objective(args: &[&str]) -> bool {
    let mut index = 1;
    while index < args.len() {
        let arg = args[index];
        if arg == "--" {
            return args.get(index + 1).is_some();
        }
        if arg.starts_with('-') {
            if code_option_takes_value(arg) && !arg.contains('=') {
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        return true;
    }
    false
}

fn code_option_takes_value(arg: &str) -> bool {
    matches!(
        arg,
        "--diff"
            | "--mode"
            | "--max-iterations"
            | "--model-token-budget"
            | "--test-command"
            | "--session-id"
            | "--turn-id"
            | "--test-analysis-json"
    )
}

fn shell_quote(input: &str) -> String {
    format!("\"{}\"", input.replace('\\', "\\\\").replace('"', "\\\""))
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
