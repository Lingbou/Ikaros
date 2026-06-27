// SPDX-License-Identifier: GPL-3.0-only

use crate::browser::run_browser_workbench_command;
use crate::code::{code_command, parse_interactive_code_command};
use crate::debug::{debug_sandbox_json_line, print_sandbox_status_for_human};
use anyhow::{Context, Result};
use ikaros_agent::chat::ChatRunOptions;
use ikaros_terminal::{clear_visible_terminal, terminal_inline};
use serde_json::json;

use crate::chat::notice::WorkbenchNotice;
use crate::chat::workbench::{
    TimelineVerbosity, api_status_human_lines, context_mentions_human_lines,
    context_status_human_lines, format_workbench_help, memory_status_human_lines,
    model_status_human_lines, print_api_status, print_context_mentions, print_context_status,
    print_diff_status, print_diff_status_for_human, print_memory_status, print_model_status,
    print_rag_status, print_replay_status, print_replay_status_for_human, print_session_summaries,
    print_slash_commands, print_tasks_status, print_tools_status, print_trace_status,
    print_trace_status_for_human, print_workbench_input_history, print_workbench_status,
    provider_status_human_lines, rag_status_human_lines, session_summaries_human_lines,
    slash_commands_human_lines, suggest_slash_command, tasks_status_human_lines,
    tools_status_human_lines, workbench_input_history_human_lines, workbench_status_human_lines,
};

use super::agent::handle_agent_command;
use super::approval::handle_approval_command;
use super::attachment::handle_attach_command;
use super::code_alias::workbench_code_alias_command;
use super::continuations::{handle_cancel_command, handle_queue_command};
use super::debug::handle_debug_command;
use super::evidence::append_workbench_evidence;
use super::gateway::handle_gateway_command;
use super::mcp::handle_mcp_command;
use super::multimodal::{handle_image_command, handle_vision_command};
use super::parse::{parse_timeline_request, parse_trace_request};
use super::provider::{handle_budget_command, handle_provider_command};
use super::screen::handle_screen_command;
use super::session::{handle_fork_command, handle_session_command};
use super::web::handle_web_command;
use super::{
    InteractiveChatRuntime, InteractiveCommandContext, clear_interactive_session,
    print_default_inline_lines, suppress_fullscreen_stdout_command,
};
use super::{available_agent_lines, available_agent_lines_for_human};

pub(in crate::chat) async fn handle_interactive_chat_command(
    input: &str,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
    options: &mut ChatRunOptions,
) -> Result<()> {
    if suppress_fullscreen_stdout_command(input, runtime)? {
        return Ok(());
    }
    let config = ctx.config;
    let paths = ctx.paths;
    let workspace = ctx.workspace;
    let usage_ledger = ctx.usage_ledger;
    let mut parts = input.split_whitespace();
    let command = parts.next().unwrap_or_default();
    match command {
        "/help" => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(slash_commands_human_lines(&[]))?;
            } else {
                println!("{}", format_workbench_help());
            }
        }
        "/commands" => {
            let args = parts.collect::<Vec<_>>();
            if runtime.default_inline_stdout() {
                print_default_inline_lines(slash_commands_human_lines(&args))?;
            } else {
                print_slash_commands(&args);
            }
        }
        "/queue" => {
            handle_queue_command(parts.collect::<Vec<_>>(), runtime)?;
        }
        "/attach" | "/attachments" => {
            handle_attach_command(parts.collect::<Vec<_>>(), runtime, workspace)?;
        }
        "/agents" => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(available_agent_lines_for_human(
                    config,
                    &runtime.agent.name,
                ))?;
            } else {
                for line in available_agent_lines(config, &runtime.agent.name) {
                    println!("{line}");
                }
            }
        }
        "/agent" => {
            handle_agent_command(parts.collect::<Vec<_>>(), config, paths, workspace, runtime)?;
        }
        "/status" => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(workbench_status_human_lines(
                    config,
                    paths,
                    workspace,
                    runtime,
                    options,
                    usage_ledger,
                )?)?;
            } else {
                print_workbench_status(config, paths, workspace, runtime, options, usage_ledger)?;
            }
        }
        "/budget" => {
            handle_budget_command(parts.collect::<Vec<_>>(), paths, runtime)?;
        }
        "/screen" => {
            handle_screen_command(parts.collect::<Vec<_>>(), ctx, runtime, options).await?;
        }
        "/history" => {
            let limit = parts
                .next()
                .map(|limit| {
                    limit
                        .parse::<usize>()
                        .with_context(|| "history limit must be a positive number")
                })
                .transpose()?
                .unwrap_or(20);
            if runtime.default_inline_stdout() {
                print_default_inline_lines(workbench_input_history_human_lines(paths, limit)?)?;
            } else {
                print_workbench_input_history(paths, limit)?;
            }
        }
        "/sessions" => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(session_summaries_human_lines(
                    config, paths, workspace, runtime, 20,
                )?)?;
            } else {
                print_session_summaries(config, paths, workspace, runtime, 20)?;
            }
        }
        "/session" => {
            handle_session_command(
                parts.collect::<Vec<_>>(),
                config,
                paths,
                workspace,
                runtime,
                options,
            )?;
        }
        "/resume" => {
            handle_session_command(
                std::iter::once("resume").chain(parts).collect::<Vec<_>>(),
                config,
                paths,
                workspace,
                runtime,
                options,
            )?;
        }
        "/clear" | "/new" => {
            let report = clear_interactive_session(runtime, options);
            clear_visible_terminal()?;
            runtime.push_notice(WorkbenchNotice::info(
                "session cleared",
                &format!(
                    "old_session={} new_session={} pending_inputs_cleared={} attachments_cleared={}",
                    terminal_inline(&report.old_session_id),
                    terminal_inline(&report.new_session_id),
                    report.pending_inputs_cleared,
                    report.attachments_cleared
                ),
            ));
            if runtime.default_inline_stdout() {
                print_default_inline_lines(vec![
                    "* Cleared".to_owned(),
                    format!("  session: {}", terminal_inline(&report.new_session_id)),
                ])?;
            } else if !runtime.fullscreen_stdout_quiet() {
                println!(
                    "session_cleared: old={} new={} pending_inputs_cleared={} attachments_cleared={}",
                    terminal_inline(&report.old_session_id),
                    terminal_inline(&report.new_session_id),
                    report.pending_inputs_cleared,
                    report.attachments_cleared
                );
            }
        }
        "/fork" => {
            handle_fork_command(parts.collect::<Vec<_>>(), runtime)?;
        }
        "/timeline" => {
            let request = parse_timeline_request(parts.collect::<Vec<_>>())?;
            if runtime.default_inline_stdout() {
                print_replay_status_for_human(
                    "timeline",
                    config,
                    paths,
                    workspace,
                    runtime,
                    TimelineVerbosity::Timeline,
                    request,
                )?;
            } else {
                print_replay_status(
                    "timeline",
                    config,
                    paths,
                    workspace,
                    runtime,
                    TimelineVerbosity::Timeline,
                    request,
                )?;
            }
        }
        "/replay" => {
            let request = parse_timeline_request(parts.collect::<Vec<_>>())?;
            if runtime.default_inline_stdout() {
                print_replay_status_for_human(
                    "replay",
                    config,
                    paths,
                    workspace,
                    runtime,
                    TimelineVerbosity::Replay,
                    request,
                )?;
            } else {
                print_replay_status(
                    "replay",
                    config,
                    paths,
                    workspace,
                    runtime,
                    TimelineVerbosity::Replay,
                    request,
                )?;
            }
        }
        "/debug" => {
            let args = parts.collect::<Vec<_>>();
            handle_debug_command(args, config, paths, workspace, runtime).await?;
        }
        "/trace" => {
            let request = parse_trace_request(parts.collect::<Vec<_>>())?;
            if runtime.default_inline_stdout() {
                print_trace_status_for_human(config, paths, workspace, runtime, request)?;
            } else {
                print_trace_status(config, paths, workspace, runtime, request)?;
            }
        }
        "/sandbox" => {
            let args = parts.collect::<Vec<_>>();
            let probe = args.first().copied() == Some("--probe");
            if !args.is_empty() && !probe {
                if runtime.default_inline_stdout() {
                    print_default_inline_lines(vec![
                        "* Sandbox".to_owned(),
                        "  usage: /sandbox [--probe]".to_owned(),
                    ])?;
                } else {
                    println!("usage: /sandbox [--probe]");
                }
            } else if runtime.default_inline_stdout() {
                print_sandbox_status_for_human(paths, workspace, Some(&runtime.agent.name), probe)
                    .await?;
            } else {
                println!(
                    "{}",
                    debug_sandbox_json_line(paths, workspace, Some(&runtime.agent.name), probe)
                        .await?
                );
                append_workbench_evidence(
                    runtime,
                    "sandbox",
                    json!({
                        "probe": probe,
                        "command": "/sandbox",
                    }),
                )?;
            }
        }
        "/mentions" => {
            let query = parts.next();
            if runtime.default_inline_stdout() {
                print_default_inline_lines(context_mentions_human_lines(workspace, query)?)?;
            } else {
                print_context_mentions(workspace, query)?;
            }
        }
        "/provider" => {
            let args = parts.collect::<Vec<_>>();
            if runtime.default_inline_stdout() && !args.iter().any(|arg| *arg == "--json") {
                print_default_inline_lines(provider_status_human_lines(paths, runtime, &args)?)?;
            } else {
                handle_provider_command(args.clone(), paths, workspace, runtime).await?;
            }
            append_workbench_evidence(runtime, "provider", json!({"args": args}))?;
        }
        "/model" => {
            if runtime.fullscreen_stdout_quiet() {
                runtime.push_notice(WorkbenchNotice::info(
                    "model",
                    "model status refreshed in the workbench",
                ));
            } else if runtime.default_inline_stdout() {
                print_default_inline_lines(model_status_human_lines(paths, runtime)?)?;
            } else {
                print_model_status(paths, runtime)?;
            }
            append_workbench_evidence(runtime, "model", json!({"args": ["inspect"]}))?;
        }
        "/gateway" => {
            let args = parts.collect::<Vec<_>>();
            handle_gateway_command(args.clone(), paths, workspace, runtime)?;
            append_workbench_evidence(runtime, "gateway", json!({"args": args}))?;
        }
        "/tasks" => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(tasks_status_human_lines(paths)?)?;
            } else {
                print_tasks_status(paths)?;
            }
            append_workbench_evidence(runtime, "tasks", json!({}))?;
        }
        "/approval" | "/approvals" => {
            handle_approval_command(
                parts.collect::<Vec<_>>(),
                paths,
                workspace,
                runtime,
                "slash_command",
            )
            .await?;
        }
        "/cancel" => {
            handle_cancel_command(parts.collect::<Vec<_>>(), runtime)?;
        }
        "/context" => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(context_status_human_lines(runtime, options)?)?;
            } else {
                print_context_status(runtime, options)?;
            }
        }
        "/memory" => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(memory_status_human_lines(config, paths, runtime)?)?;
            } else {
                print_memory_status(config, paths, runtime)?;
            }
        }
        "/rag" => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(rag_status_human_lines(config, paths, options))?;
            } else {
                print_rag_status(config, paths, options);
            }
        }
        "/tools" => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(tools_status_human_lines(
                    ctx.registry,
                    &runtime.agent,
                )?)?;
            } else {
                print_tools_status(ctx.registry, &runtime.agent)?;
            }
            append_workbench_evidence(runtime, "tools", json!({"agent": &runtime.agent.name}))?;
        }
        "/mcp" => {
            let args = parts.collect::<Vec<_>>();
            handle_mcp_command(args, ctx, runtime).await?;
        }
        "/api" => {
            let args = parts.collect::<Vec<_>>();
            if args.is_empty() || args == ["status"] {
                if runtime.default_inline_stdout() {
                    print_default_inline_lines(api_status_human_lines(config))?;
                } else {
                    print_api_status(config);
                }
                append_workbench_evidence(runtime, "api", json!({"args": args}))?;
            } else {
                if runtime.default_inline_stdout() {
                    print_default_inline_lines(vec![
                        "* API".to_owned(),
                        "  usage: /api status".to_owned(),
                        "  server: start with `ikaros api serve ...`".to_owned(),
                    ])?;
                } else {
                    println!("usage: /api status");
                    println!("api_policy: start with explicit top-level `ikaros api serve ...`");
                }
            }
        }
        "/browser" => {
            let args = parts.collect::<Vec<_>>();
            run_browser_workbench_command(&runtime.session, paths, &args).await?;
            append_workbench_evidence(runtime, "browser", json!({"args": args}))?;
        }
        "/web" => {
            let args = parts.collect::<Vec<_>>();
            handle_web_command(args.clone(), ctx, runtime).await?;
            append_workbench_evidence(runtime, "web", json!({"args": args}))?;
        }
        "/vision" => {
            let args = parts.collect::<Vec<_>>();
            handle_vision_command(args.clone(), ctx, runtime).await?;
            append_workbench_evidence(runtime, "vision", json!({"args": args}))?;
        }
        "/image" => {
            let args = parts.collect::<Vec<_>>();
            handle_image_command(args.clone(), ctx, runtime).await?;
            append_workbench_evidence(runtime, "image", json!({"args": args}))?;
        }
        "/diff" => {
            if runtime.default_inline_stdout() {
                print_diff_status_for_human(runtime, workspace).await?;
            } else {
                print_diff_status(runtime, workspace).await?;
            }
        }
        "/code" => {
            let command_line = input
                .strip_prefix("/code")
                .map(str::trim)
                .unwrap_or_default();
            if command_line.is_empty() {
                if runtime.default_inline_stdout() {
                    print_default_inline_lines(vec![
                        "* Code".to_owned(),
                        "  run: /code <plan|apply|test|review|rollback> ...".to_owned(),
                    ])?;
                } else {
                    println!("usage: /code <plan|apply|test|review|rollback> ...");
                }
            } else {
                let command = parse_interactive_code_command(command_line)
                    .with_context(|| "failed to parse /code command")?;
                code_command(command, paths, workspace, Some(&runtime.agent.name)).await?;
            }
        }
        "/review" | "/rollback" => {
            let command_line = workbench_code_alias_command(command, input)?;
            let parsed = parse_interactive_code_command(&command_line)
                .with_context(|| format!("failed to parse {command} command"))?;
            code_command(parsed, paths, workspace, Some(&runtime.agent.name)).await?;
        }
        _ => {
            if runtime.fullscreen_stdout_quiet() {
                let suggestion = suggest_slash_command(command)
                    .map(|suggestion| format!(" Did you mean {suggestion}?"))
                    .unwrap_or_default();
                runtime.push_notice(WorkbenchNotice::error(
                    "unknown command",
                    &format!(
                        "{} is not a known slash command.{}",
                        terminal_inline(command),
                        suggestion
                    ),
                ));
            } else if runtime.default_inline_stdout() {
                let mut lines = vec![
                    "* Unknown command".to_owned(),
                    format!("  command: {}", terminal_inline(command)),
                    "  help: /help".to_owned(),
                ];
                if let Some(suggestion) = suggest_slash_command(command) {
                    lines.push(format!("  suggestion: {suggestion}"));
                }
                print_default_inline_lines(lines)?;
            } else {
                println!(
                    "unknown command: {}. Type /help for commands.",
                    terminal_inline(command)
                );
                if let Some(suggestion) = suggest_slash_command(command) {
                    println!("did_you_mean: {suggestion}");
                }
            }
        }
    }
    Ok(())
}
