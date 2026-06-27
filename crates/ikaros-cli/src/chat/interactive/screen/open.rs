// SPDX-License-Identifier: GPL-3.0-only

mod code;
mod command_parse;
mod debug;
mod explicit;
mod outcome;
mod selected;
mod timeline;

use crate::browser::run_browser_workbench_command;
use crate::gateway::{run_gateway_adapter_workbench_command, run_gateway_daemon_workbench_command};
use anyhow::Result;
use ikaros_agent::chat::ChatRunOptions;

use super::super::approval::handle_approval_command;
use super::super::attachment::handle_attach_command;
use super::super::continuations::{handle_cancel_command, handle_queue_command};
use super::super::multimodal::{handle_image_command, handle_vision_command};
use super::super::provider::{handle_budget_command, handle_provider_command};
use super::super::web::handle_web_command;
use super::super::{InteractiveChatRuntime, InteractiveCommandContext};
use super::{
    handle_screen_selected_approval_action, handle_screen_selected_continuation_action,
    handle_screen_selected_input_action,
};
use crate::chat::notice::{WorkbenchNotice, WorkbenchNoticeKind};
use crate::chat::workbench::{
    TimelineVerbosity, WorkbenchScreenApprovalAction, WorkbenchScreenContinuationAction,
    WorkbenchScreenInputAction, command_requires_explicit_action, format_workbench_help,
    print_api_status_for_human, print_context_status, print_context_status_for_human,
    print_diff_status, print_diff_status_for_human, print_gateway_status,
    print_gateway_status_for_human, print_mcp_status, print_mcp_status_for_human,
    print_memory_status, print_memory_status_for_human, print_model_status,
    print_model_status_for_human, print_provider_status_for_human, print_rag_status,
    print_rag_status_for_human, print_slash_commands, print_slash_commands_for_human,
    print_tools_status, print_tools_status_for_human, print_workbench_status,
    print_workbench_status_for_human,
};
use code::execute_screen_code_command;
use command_parse::{command_tail, screen_budget_command_resumes_pending_inputs};
use debug::execute_screen_open_debug_command;
use explicit::execute_confirmed_explicit_command;
use outcome::ScreenOpenCommandStatus;
pub(super) use selected::handle_screen_open_selected_action;
use timeline::{execute_replay_status_command, execute_trace_status_command};

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
    if super::super::suppress_fullscreen_stdout_command(command, runtime)? {
        return Ok(ScreenOpenCommandStatus::Executed);
    }
    let use_human_output = runtime.default_inline_stdout() && !runtime.screen_state.raw_mode();
    let mut parts = command.split_whitespace();
    let status = match (parts.next(), parts.next()) {
        (Some("/timeline"), _) => execute_replay_status_command(
            "timeline",
            TimelineVerbosity::Timeline,
            command,
            ctx,
            runtime,
            use_human_output,
        )?,
        (Some("/replay"), _) => execute_replay_status_command(
            "replay",
            TimelineVerbosity::Replay,
            command,
            ctx,
            runtime,
            use_human_output,
        )?,
        (Some("/trace"), _) => {
            execute_trace_status_command(command, ctx, runtime, use_human_output)?
        }
        (Some("/debug"), subcommand) => {
            let args = parts.collect::<Vec<_>>();
            execute_screen_open_debug_command(subcommand, &args, ctx, runtime, use_human_output)
                .await?
        }
        (Some("/context"), _) => {
            if use_human_output {
                print_context_status_for_human(runtime, options)?;
            } else {
                print_context_status(runtime, options)?;
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/memory"), _) => {
            if use_human_output {
                print_memory_status_for_human(ctx.config, ctx.paths, runtime)?;
            } else {
                print_memory_status(ctx.config, ctx.paths, runtime)?;
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/rag"), _) => {
            if use_human_output {
                print_rag_status_for_human(ctx.config, ctx.paths, options);
            } else {
                print_rag_status(ctx.config, ctx.paths, options);
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/diff"), _) => {
            if use_human_output {
                print_diff_status_for_human(runtime, ctx.workspace).await?;
            } else {
                print_diff_status(runtime, ctx.workspace).await?;
            }
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
            } else if use_human_output {
                print_model_status_for_human(ctx.paths, runtime)?;
            } else {
                print_model_status(ctx.paths, runtime)?;
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/status"), _) => {
            if use_human_output {
                print_workbench_status_for_human(
                    ctx.config,
                    ctx.paths,
                    ctx.workspace,
                    runtime,
                    options,
                    ctx.usage_ledger,
                )?;
            } else {
                print_workbench_status(
                    ctx.config,
                    ctx.paths,
                    ctx.workspace,
                    runtime,
                    options,
                    ctx.usage_ledger,
                )?;
            }
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
            if use_human_output {
                print_tools_status_for_human(ctx.registry, &runtime.agent)?;
            } else {
                print_tools_status(ctx.registry, &runtime.agent)?;
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/commands"), _) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            if use_human_output {
                print_slash_commands_for_human(&args);
            } else {
                print_slash_commands(&args);
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/mcp"), None) | (Some("/mcp"), Some("status")) => {
            if use_human_output {
                print_mcp_status_for_human(ctx.config);
            } else {
                print_mcp_status(ctx.config);
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/api"), None) | (Some("/api"), Some("status")) => {
            if use_human_output {
                print_api_status_for_human(ctx.config);
            } else {
                super::super::print_api_status(ctx.config);
            }
            ScreenOpenCommandStatus::Executed
        }
        (Some("/browser"), None)
        | (Some("/browser"), Some("status" | "list" | "supervisor-status" | "supervisor")) => {
            let args = command.split_whitespace().skip(1).collect::<Vec<_>>();
            run_browser_workbench_command(&runtime.session, ctx.paths, &args).await?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/web"), None) | (Some("/web"), Some("help" | "--help")) => {
            handle_web_command(
                command.split_whitespace().skip(1).collect::<Vec<_>>(),
                ctx,
                runtime,
            )
            .await?;
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
            handle_vision_command(
                command.split_whitespace().skip(1).collect::<Vec<_>>(),
                ctx,
                runtime,
            )
            .await?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/vision"), Some("describe")) if parts.next().is_none() => {
            handle_vision_command(
                command.split_whitespace().skip(1).collect::<Vec<_>>(),
                ctx,
                runtime,
            )
            .await?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/image"), None) | (Some("/image"), Some("help" | "--help")) => {
            handle_image_command(
                command.split_whitespace().skip(1).collect::<Vec<_>>(),
                ctx,
                runtime,
            )
            .await?;
            ScreenOpenCommandStatus::Executed
        }
        (Some("/image"), Some("generate")) if parts.next().is_none() => {
            handle_image_command(
                command.split_whitespace().skip(1).collect::<Vec<_>>(),
                ctx,
                runtime,
            )
            .await?;
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
            } else if use_human_output && !args.iter().any(|arg| *arg == "--json") {
                print_provider_status_for_human(ctx.paths, runtime, &args)?;
                ScreenOpenCommandStatus::Executed
            } else {
                handle_provider_command(args, ctx.paths, ctx.workspace, runtime).await?;
                ScreenOpenCommandStatus::Executed
            }
        }
        (Some("/gateway"), Some("daemon")) => {
            let args = command.split_whitespace().skip(2).collect::<Vec<_>>();
            if matches!(args.as_slice(), [] | ["status"]) {
                if use_human_output {
                    print_gateway_status_for_human(ctx.paths)?;
                } else {
                    run_gateway_daemon_workbench_command(
                        &args,
                        ctx.paths,
                        ctx.workspace,
                        Some(&runtime.agent.name),
                    )?;
                }
                ScreenOpenCommandStatus::Executed
            } else {
                ScreenOpenCommandStatus::ExplicitActionRequired
            }
        }
        (Some("/gateway"), Some("adapter")) => {
            let args = command.split_whitespace().skip(2).collect::<Vec<_>>();
            if matches!(args.as_slice(), [] | ["list"] | ["status"]) {
                if use_human_output {
                    print_gateway_status_for_human(ctx.paths)?;
                } else {
                    run_gateway_adapter_workbench_command(&args, ctx.paths)?;
                }
                ScreenOpenCommandStatus::Executed
            } else {
                ScreenOpenCommandStatus::ExplicitActionRequired
            }
        }
        (Some("/gateway"), _) => {
            if use_human_output {
                print_gateway_status_for_human(ctx.paths)?;
            } else {
                print_gateway_status(ctx.paths)?;
            }
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
            if !runtime.fullscreen_stdout_quiet() && !use_human_output {
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
            handle_queue_command(
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
