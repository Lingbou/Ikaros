// SPDX-License-Identifier: GPL-3.0-only

mod agent;
mod approval;
mod attachment;
mod continuations;
mod evidence;
mod parse;
mod provider;
mod screen;
mod session;
mod status;

use crate::browser::run_browser_workbench_command;
use crate::code::{code_command, parse_interactive_code_command};
use crate::debug::{
    debug_dump_json_line, debug_insights_json_line, debug_logs_json_line,
    debug_memory_lifecycle_json_line, debug_readiness_json_line, debug_sandbox_json_line,
    debug_state_db_json_line, print_sandbox_status_for_human,
};
use crate::gateway::{run_gateway_adapter_workbench_command, run_gateway_daemon_workbench_command};
use crate::image::{ImageCommand, ImageGenerateArgs, ImageResponseFormat, image_command};
use crate::mcp::run_mcp_http_call;
use crate::vision::VisionDescribeArgs;
use anyhow::{Context, Result, anyhow};
use ikaros_core::{
    IkarosConfig, IkarosPaths, ModelConfig, RemoteProviderConfig, ResolvedAgentProfile,
    redact_json, redact_secrets,
};
use ikaros_harness::{ExecutionSession, SkillRegistry};
use ikaros_models::{ModelContentBlock, ModelProvider, ModelRequestOptions, ModelUsageLedger};
use ikaros_runtime::{ChatRunOptions, new_chat_session_id};
use serde_json::json;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use agent::handle_agent_command;
use approval::handle_approval_command;
use attachment::handle_attach_command;
#[cfg(test)]
pub(super) use continuations::{
    WorkbenchCancelTarget, cancel_selected_screen_continuation, cancel_session_continuations,
    continuations_json_line,
};
use continuations::{
    handle_cancel_command, handle_queue_command, print_workbench_continuation_status,
};
use evidence::append_workbench_evidence;
use parse::{
    parse_debug_dump_recent_logs, parse_debug_logs_args, parse_timeline_request,
    parse_trace_request,
};
pub(super) use provider::build_interactive_model_provider;
use provider::{handle_budget_command, handle_provider_command};
use screen::handle_screen_command;
use session::{handle_fork_command, handle_session_command};
pub(super) use status::{
    InteractiveChatStatusInput, available_agent_lines, available_agent_lines_for_human,
    format_interactive_chat_status,
};

use super::notice::WorkbenchNotice;
use super::progress::WorkbenchProgressSnapshot;
use super::slash::{SlashCommandFullscreenEffect, slash_command_fullscreen_effect};
use super::workbench::{
    TimelineVerbosity, WorkbenchScreenState, api_status_human_lines, apply_workbench_screen_args,
    context_mentions_human_lines, context_status_human_lines, format_workbench_help,
    gateway_status_human_lines, mcp_status_human_lines, memory_status_human_lines,
    model_status_human_lines, print_api_status, print_context_mentions, print_context_status,
    print_diff_status, print_diff_status_for_human, print_gateway_status, print_mcp_status,
    print_memory_status, print_model_status, print_rag_status, print_replay_status,
    print_replay_status_for_human, print_session_summaries, print_slash_commands,
    print_tasks_status, print_tools_status, print_trace_status, print_trace_status_for_human,
    print_workbench_input_history, print_workbench_status, provider_status_human_lines,
    rag_status_human_lines, session_summaries_human_lines, slash_commands_human_lines,
    suggest_slash_command, tasks_status_human_lines, terminal_inline, tools_status_human_lines,
    workbench_input_history_human_lines, workbench_status_human_lines,
};

pub(super) struct InteractiveChatRuntime {
    pub(super) agent: ResolvedAgentProfile,
    pub(super) agent_id: String,
    pub(super) state_dir: PathBuf,
    pub(super) workspace: PathBuf,
    pub(super) model_config: ModelConfig,
    pub(super) model_provider: RemoteProviderConfig,
    pub(super) provider: Box<dyn ModelProvider>,
    pub(super) session: ExecutionSession,
    pub(super) chat_session_id: String,
    pub(super) request_options: ModelRequestOptions,
    pub(super) pending_inputs: VecDeque<String>,
    pub(super) pending_content_blocks: Vec<ModelContentBlock>,
    pub(super) screen_state: WorkbenchScreenState,
    pub(super) default_inline_ui: bool,
    pub(super) last_progress: Option<WorkbenchProgressSnapshot>,
    pub(super) notices: VecDeque<WorkbenchNotice>,
    pub(super) pending_input_drain_requested: bool,
}

impl InteractiveChatRuntime {
    pub(super) fn fullscreen_stdout_quiet(&self) -> bool {
        false
    }

    pub(super) fn default_inline_stdout(&self) -> bool {
        use std::io::IsTerminal;

        should_emit_default_inline_stdout(self.default_inline_ui, std::io::stdout().is_terminal())
    }

    pub(super) fn machine_stdout_quiet(&self) -> bool {
        self.fullscreen_stdout_quiet() || self.default_inline_stdout()
    }

    pub(super) fn push_notice(&mut self, notice: WorkbenchNotice) {
        const MAX_NOTICES: usize = 24;
        self.notices.push_back(notice);
        while self.notices.len() > MAX_NOTICES {
            self.notices.pop_front();
        }
    }

    pub(super) fn request_pending_input_drain(&mut self) {
        self.pending_input_drain_requested = true;
    }

    pub(super) fn take_pending_input_drain_request(&mut self) -> bool {
        let requested = self.pending_input_drain_requested;
        self.pending_input_drain_requested = false;
        requested
    }
}

pub(in crate::chat) fn should_emit_default_inline_stdout(
    default_inline_ui: bool,
    _stdout_is_terminal: bool,
) -> bool {
    default_inline_ui
}

pub(super) struct InteractiveCommandContext<'a> {
    pub(super) config: &'a IkarosConfig,
    pub(super) paths: &'a IkarosPaths,
    pub(super) workspace: &'a Path,
    pub(super) usage_ledger: &'a ModelUsageLedger,
    pub(super) registry: &'a SkillRegistry,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ClearInteractiveSessionReport {
    pub(super) old_session_id: String,
    pub(super) new_session_id: String,
    pub(super) pending_inputs_cleared: usize,
    pub(super) attachments_cleared: usize,
}

pub(super) fn clear_interactive_session(
    runtime: &mut InteractiveChatRuntime,
    options: &mut ChatRunOptions,
) -> ClearInteractiveSessionReport {
    let old_session_id = runtime.chat_session_id.clone();
    let new_session_id = new_chat_session_id();
    let pending_inputs_cleared = runtime.pending_inputs.len();
    let attachments_cleared = runtime.pending_content_blocks.len();

    runtime.chat_session_id = new_session_id.clone();
    options.session_id = Some(new_session_id.clone());
    runtime.pending_inputs.clear();
    runtime.pending_content_blocks.clear();
    runtime.screen_state = WorkbenchScreenState::default();
    runtime.last_progress = None;
    runtime.notices.clear();
    runtime.pending_input_drain_requested = false;

    ClearInteractiveSessionReport {
        old_session_id,
        new_session_id,
        pending_inputs_cleared,
        attachments_cleared,
    }
}

fn print_default_inline_lines(lines: Vec<String>) -> Result<()> {
    super::terminal::print_inline_history_lines(&lines)
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

pub(super) async fn handle_interactive_chat_command(
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
            super::terminal::clear_visible_terminal()?;
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
                    "• Cleared".to_owned(),
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
            if runtime.default_inline_stdout() {
                print_default_inline_lines(debug_status_human_lines(&args))?;
            } else if args.first().copied() == Some("readiness") {
                println!("readiness: see readiness_json for structured MVP status");
                println!(
                    "{}",
                    debug_readiness_json_line(paths, workspace, Some(&runtime.agent.name))?
                );
            } else if args.first().copied() == Some("sandbox") {
                let probe = args.get(1).copied() == Some("--probe");
                println!(
                    "{}",
                    debug_sandbox_json_line(paths, workspace, Some(&runtime.agent.name), probe)
                        .await?
                );
            } else if args.first().copied() == Some("logs") {
                let (source, page, page_size) = parse_debug_logs_args(&args[1..])?;
                println!("{}", debug_logs_json_line(paths, source, page, page_size)?);
            } else if args.first().copied() == Some("insights") {
                println!(
                    "{}",
                    debug_insights_json_line(paths, workspace, Some(&runtime.agent.name))?
                );
            } else if args.first().copied() == Some("dump") {
                let recent_logs = parse_debug_dump_recent_logs(&args[1..])?;
                println!(
                    "{}",
                    debug_dump_json_line(paths, workspace, Some(&runtime.agent.name), recent_logs)?
                );
            } else if args.first().copied() == Some("state-db")
                || args.first().copied() == Some("state_db")
            {
                println!(
                    "{}",
                    debug_state_db_json_line(paths, workspace, Some(&runtime.agent.name))?
                );
            } else if args.first().copied() == Some("continuations") {
                print_workbench_continuation_status(runtime)?;
            } else if args.first().copied() == Some("memory-lifecycle")
                || args.first().copied() == Some("memory_lifecycle")
            {
                let (session_id, turn_id) =
                    parse_debug_memory_lifecycle_args(&args[1..], &runtime.chat_session_id);
                println!(
                    "{}",
                    debug_memory_lifecycle_json_line(
                        paths,
                        workspace,
                        Some(&runtime.agent.name),
                        &session_id,
                        turn_id.as_deref(),
                    )?
                );
            } else {
                print_replay_status(
                    "debug",
                    config,
                    paths,
                    workspace,
                    runtime,
                    TimelineVerbosity::Debug,
                    parse_timeline_request(args)?,
                )?;
            }
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
                        "• Sandbox".to_owned(),
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
                        "• API".to_owned(),
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
                        "• Code".to_owned(),
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
                    "• Unknown command".to_owned(),
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

fn print_debug_status_for_human(args: &[&str]) {
    for line in debug_status_human_lines(args) {
        println!("{line}");
    }
}

fn debug_status_human_lines(args: &[&str]) -> Vec<String> {
    let mut lines = vec!["• Debug".to_owned()];
    match args.first().copied() {
        Some("readiness") => {
            lines.push(
                "  readiness: use `ikaros debug readiness` for the structured report".to_owned(),
            );
            lines.push("  status: use /status for the current terminal UI summary".to_owned());
        }
        Some("sandbox") => {
            lines.push(
                "  sandbox: use /sandbox [--probe] for the human diagnostics view".to_owned(),
            );
        }
        Some("logs") => {
            lines.push("  logs: use `ikaros debug logs` for the structured log report".to_owned());
        }
        Some("insights") => {
            lines.push(
                "  insights: use `ikaros debug insights` for the structured report".to_owned(),
            );
        }
        Some("dump") => {
            lines.push(
                "  dump: use `ikaros debug dump` to write explicit debug artifacts".to_owned(),
            );
        }
        Some("state-db") | Some("state_db") => {
            lines.push(
                "  state db: use `ikaros debug state-db` for the structured report".to_owned(),
            );
        }
        Some("continuations") => {
            lines.push("  continuations: use /queue and /cancel for the human controls".to_owned());
        }
        Some(other) => {
            lines.push(format!("  filter: {}", terminal_inline(other)));
            lines.push(
                "  timeline: use /timeline, /replay, or /trace for session inspection".to_owned(),
            );
        }
        None => {
            lines.push(
                "  timeline: use /timeline, /replay, or /trace for session inspection".to_owned(),
            );
            lines.push(
                "  readiness: use `ikaros debug readiness` for structured diagnostics".to_owned(),
            );
        }
    }
    lines
}

async fn handle_vision_command(
    args: Vec<&str>,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    match args.as_slice() {
        ["describe", rest @ ..] => {
            let describe = parse_vision_describe_args(rest)?;
            let mut input = json!({
                "image": describe.image,
                "prompt": describe.prompt,
            });
            if let Some(detail) = describe.detail {
                input["detail"] = json!(detail);
            }
            let result = runtime
                .session
                .execute_skill(ctx.registry, "vision_describe", input)
                .await?;
            emit_interactive_vision_result(&result, runtime)?;
        }
        ["help"] | ["--help"] | [] => emit_vision_usage(runtime)?,
        _ => emit_vision_usage(runtime)?,
    }
    Ok(())
}

fn parse_vision_describe_args(args: &[&str]) -> Result<VisionDescribeArgs> {
    let mut image: Option<String> = None;
    let mut prompt =
        "Describe this image. Mention visible text, UI state, objects, and anything relevant to debugging or understanding the scene."
            .to_owned();
    let mut detail = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--prompt" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /vision describe <image> --prompt TEXT"))?;
                prompt = (*value).to_owned();
                index += 2;
            }
            "--detail" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /vision describe <image> --detail low|high|auto")
                })?;
                detail = Some((*value).to_owned());
                index += 2;
            }
            value if image.is_none() => {
                image = Some(value.to_owned());
                index += 1;
            }
            value => return Err(anyhow!("unexpected /vision argument: {value}")),
        }
    }
    Ok(VisionDescribeArgs {
        image: image.ok_or_else(|| anyhow!("usage: /vision describe <image>"))?,
        prompt,
        detail,
    })
}

pub(super) fn print_vision_usage() {
    println!(
        "vision_usage: /vision describe <image-path|url|data-url> [--prompt TEXT] [--detail low|high|auto]"
    );
}

fn emit_vision_usage(runtime: &mut InteractiveChatRuntime) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        runtime.push_notice(WorkbenchNotice::info(
            "vision",
            "usage is available in the fullscreen vision cell",
        ));
    } else if runtime.default_inline_stdout() {
        print_default_inline_lines(vision_usage_human_lines())?;
    } else {
        print_vision_usage();
    }
    Ok(())
}

fn vision_usage_human_lines() -> Vec<String> {
    vec![
        "• Vision".to_owned(),
        "  usage: /vision describe <image-path|url|data-url>".to_owned(),
        "  options: --prompt TEXT, --detail low|high|auto".to_owned(),
    ]
}

async fn handle_image_command(
    args: Vec<&str>,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    match args.as_slice() {
        ["generate", rest @ ..] => {
            let generate = parse_image_generate_args(rest)?;
            if generate.output_dir.is_some() {
                image_command(
                    ImageCommand::Generate(generate),
                    ctx.paths,
                    ctx.workspace,
                    Some(&runtime.agent.name),
                )
                .await?;
            } else {
                let input = image_generate_skill_input(&generate);
                let result = runtime
                    .session
                    .execute_skill(ctx.registry, "image_generate", input)
                    .await?;
                emit_interactive_image_result(&result, runtime)?;
            }
        }
        ["help"] | ["--help"] | [] => emit_image_usage(runtime)?,
        _ => emit_image_usage(runtime)?,
    }
    Ok(())
}

fn parse_image_generate_args(args: &[&str]) -> Result<ImageGenerateArgs> {
    let mut prompt_tokens = Vec::new();
    let mut model = None;
    let mut size = "1024x1024".to_owned();
    let mut n = 1;
    let mut response_format = ImageResponseFormat::Url;
    let mut quality = None;
    let mut style = None;
    let mut output_dir = None;
    let mut output_format = "png".to_owned();
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--prompt" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate --prompt TEXT"))?;
                prompt_tokens.push((*value).to_owned());
                index += 2;
            }
            "--model" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --model MODEL"))?;
                model = Some((*value).to_owned());
                index += 2;
            }
            "--size" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --size 1024x1024"))?;
                size = (*value).to_owned();
                index += 2;
            }
            "--n" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --n N"))?;
                n = value
                    .parse::<u32>()
                    .with_context(|| "--n must be a positive integer")?;
                index += 2;
            }
            "--response-format" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /image generate <prompt> --response-format url|b64_json")
                })?;
                response_format = parse_image_response_format(value)?;
                index += 2;
            }
            "--quality" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --quality QUALITY"))?;
                quality = Some((*value).to_owned());
                index += 2;
            }
            "--style" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --style STYLE"))?;
                style = Some((*value).to_owned());
                index += 2;
            }
            "--output-dir" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /image generate <prompt> --output-dir PATH"))?;
                output_dir = Some(PathBuf::from(value));
                index += 2;
            }
            "--output-format" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /image generate <prompt> --output-format png")
                })?;
                output_format = (*value).to_owned();
                index += 2;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /image generate <prompt> [--model MODEL] [--size 1024x1024] [--n N] [--response-format url|b64_json] [--output-dir PATH]"
                ));
            }
            value if value.starts_with("--") => {
                return Err(anyhow!("unknown /image generate argument: {value}"));
            }
            value => {
                prompt_tokens.push(value.to_owned());
                index += 1;
            }
        }
    }
    let prompt = prompt_tokens.join(" ");
    if prompt.trim().is_empty() {
        return Err(anyhow!("usage: /image generate <prompt>"));
    }
    Ok(ImageGenerateArgs {
        prompt,
        model,
        size,
        n,
        response_format,
        quality,
        style,
        output_dir,
        output_format,
    })
}

fn parse_image_response_format(value: &str) -> Result<ImageResponseFormat> {
    match value.trim().to_ascii_lowercase().as_str() {
        "url" => Ok(ImageResponseFormat::Url),
        "b64" | "b64_json" | "b64-json" => Ok(ImageResponseFormat::B64Json),
        _ => Err(anyhow!("--response-format must be url or b64_json")),
    }
}

pub(super) fn print_image_usage() {
    println!(
        "image_usage: /image generate <prompt> [--model MODEL] [--size 1024x1024] [--n N] [--response-format url|b64_json] [--quality VALUE] [--style VALUE] [--output-dir PATH]"
    );
}

fn emit_image_usage(runtime: &mut InteractiveChatRuntime) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        runtime.push_notice(WorkbenchNotice::info(
            "image",
            "usage is available in the fullscreen image cell",
        ));
    } else if runtime.default_inline_stdout() {
        print_default_inline_lines(image_usage_human_lines())?;
    } else {
        print_image_usage();
    }
    Ok(())
}

fn image_usage_human_lines() -> Vec<String> {
    vec![
        "• Image".to_owned(),
        "  usage: /image generate <prompt>".to_owned(),
        "  options: --model MODEL, --size 1024x1024, --n N".to_owned(),
        "  output: add --output-dir PATH to save files directly".to_owned(),
    ]
}

fn image_generate_skill_input(args: &ImageGenerateArgs) -> serde_json::Value {
    let mut input = json!({
        "prompt": &args.prompt,
        "size": &args.size,
        "n": args.n,
        "response_format": match args.response_format {
            ImageResponseFormat::Url => "url",
            ImageResponseFormat::B64Json => "b64_json",
        },
    });
    if let Some(model) = args.model.as_deref() {
        input["model"] = json!(model);
    }
    if let Some(quality) = args.quality.as_deref() {
        input["quality"] = json!(quality);
    }
    if let Some(style) = args.style.as_deref() {
        input["style"] = json!(style);
    }
    input
}

fn emit_interactive_vision_result(
    result: &ikaros_core::ToolResult,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        let model = result
            .output
            .get("model")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let content = result
            .output
            .get("content")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        runtime.push_notice(WorkbenchNotice::info(
            "vision result",
            &format!(
                "ok={} model={} summary={} content={}",
                result.ok,
                terminal_inline(model),
                terminal_inline(&result.summary),
                terminal_inline(content)
            ),
        ));
        return Ok(());
    }
    if runtime.default_inline_stdout() {
        let mut lines = vec![
            "• Vision".to_owned(),
            format!("  ok: {}", result.ok),
            format!("  summary: {}", terminal_inline(&result.summary)),
        ];
        if let Some(model) = result
            .output
            .get("model")
            .and_then(serde_json::Value::as_str)
        {
            lines.push(format!("  model: {}", terminal_inline(model)));
        }
        if let Some(content) = result
            .output
            .get("content")
            .and_then(serde_json::Value::as_str)
        {
            lines.push("  content:".to_owned());
            for line in super::render_terminal_markdown(content).lines() {
                lines.push(format!("    {line}"));
            }
        }
        print_default_inline_lines(lines)?;
        return Ok(());
    }
    println!(
        "vision_result: ok={} summary={}",
        result.ok,
        terminal_inline(&result.summary)
    );
    if let Some(model) = result
        .output
        .get("model")
        .and_then(serde_json::Value::as_str)
    {
        println!("vision_model: {}", terminal_inline(model));
    }
    if let Some(content) = result
        .output
        .get("content")
        .and_then(serde_json::Value::as_str)
    {
        println!(
            "vision_content: {}",
            super::render_terminal_markdown(content)
        );
    }
    if let Some(usage) = result.output.get("usage") {
        println!(
            "vision_usage: {}",
            serde_json::to_string(&redact_json(usage.clone()))?
        );
    }
    println!(
        "vision_json: {}",
        serde_json::to_string(&redact_json(result.output.clone()))?
    );
    Ok(())
}

fn emit_interactive_image_result(
    result: &ikaros_core::ToolResult,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        let model = result
            .output
            .get("model")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown");
        let count = result
            .output
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        runtime.push_notice(WorkbenchNotice::info(
            "image result",
            &format!(
                "ok={} model={} count={} summary={}",
                result.ok,
                terminal_inline(model),
                count,
                terminal_inline(&result.summary)
            ),
        ));
        return Ok(());
    }
    if runtime.default_inline_stdout() {
        let mut lines = vec![
            "• Image".to_owned(),
            format!("  ok: {}", result.ok),
            format!("  summary: {}", terminal_inline(&result.summary)),
        ];
        if let Some(model) = result
            .output
            .get("model")
            .and_then(serde_json::Value::as_str)
        {
            lines.push(format!("  model: {}", terminal_inline(model)));
        }
        if let Some(count) = result
            .output
            .get("count")
            .and_then(serde_json::Value::as_u64)
        {
            lines.push(format!("  images: {count}"));
        }
        print_default_inline_lines(lines)?;
        return Ok(());
    }
    println!(
        "image_result: ok={} summary={}",
        result.ok,
        terminal_inline(&result.summary)
    );
    if let Some(model) = result
        .output
        .get("model")
        .and_then(serde_json::Value::as_str)
    {
        println!("image_model: {}", terminal_inline(model));
    }
    if let Some(count) = result
        .output
        .get("count")
        .and_then(serde_json::Value::as_u64)
    {
        println!("image_count: {count}");
    }
    println!(
        "image_json: {}",
        serde_json::to_string(&redact_json(result.output.clone()))?
    );
    Ok(())
}

async fn handle_web_command(
    args: Vec<&str>,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    let Some(command) = args.first().copied() else {
        emit_web_usage(runtime)?;
        return Ok(());
    };
    let (skill, input) = match command {
        "search" => ("web_search", parse_web_search_input(&args[1..])?),
        "extract" => ("web_extract", parse_web_extract_input(&args[1..])?),
        "help" | "--help" => {
            emit_web_usage(runtime)?;
            return Ok(());
        }
        value => {
            if runtime.fullscreen_stdout_quiet() {
                runtime.push_notice(WorkbenchNotice::error(
                    "web command",
                    &format!("unsupported command={}", terminal_inline(value)),
                ));
            } else if runtime.default_inline_stdout() {
                print_default_inline_lines(vec![
                    "• Web".to_owned(),
                    format!("  unsupported command: {}", terminal_inline(value)),
                    "  usage: /web search <query> | /web extract <url>".to_owned(),
                ])?;
            } else {
                println!(
                    "web_usage_error: unsupported command={}",
                    terminal_inline(value)
                );
                print_web_usage();
            }
            return Ok(());
        }
    };
    let result = runtime
        .session
        .execute_skill(ctx.registry, skill, input)
        .await?;
    if runtime.fullscreen_stdout_quiet() {
        runtime.push_notice(WorkbenchNotice::info(
            "web result",
            &format!(
                "skill={} ok={} summary={}",
                skill,
                result.ok,
                terminal_inline(&result.summary)
            ),
        ));
        return Ok(());
    }
    if runtime.default_inline_stdout() {
        print_default_inline_lines(vec![
            "• Web".to_owned(),
            format!("  ok: {}", result.ok),
            format!("  summary: {}", terminal_inline(&result.summary)),
        ])?;
        return Ok(());
    }
    println!(
        "web_result: ok={} summary={}",
        result.ok,
        terminal_inline(&result.summary)
    );
    println!(
        "web_json: {}",
        serde_json::to_string(&redact_json(result.output.clone()))?
    );
    Ok(())
}

fn parse_web_search_input(args: &[&str]) -> Result<serde_json::Value> {
    let mut query = Vec::new();
    let mut max_results: Option<usize> = None;
    let mut endpoint: Option<String> = None;
    let mut provider: Option<String> = None;
    let mut api_key: Option<String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--max-results" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web search <query> --max-results N"))?;
                max_results = Some(
                    value
                        .parse::<usize>()
                        .with_context(|| "--max-results must be a positive integer")?,
                );
                index += 2;
            }
            "--endpoint" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web search <query> --endpoint URL"))?;
                endpoint = Some((*value).to_owned());
                index += 2;
            }
            "--provider" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web search <query> --provider PROVIDER"))?;
                provider = Some((*value).to_owned());
                index += 2;
            }
            "--api-key" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web search <query> --api-key KEY"))?;
                api_key = Some((*value).to_owned());
                index += 2;
            }
            "help" | "--help" => {
                print_web_usage();
                index += 1;
            }
            value => {
                query.push(value);
                index += 1;
            }
        }
    }
    if query.is_empty() {
        return Err(anyhow!("usage: /web search <query> [--max-results N]"));
    }
    let mut input = json!({ "query": query.join(" ") });
    if let Some(max_results) = max_results {
        input["max_results"] = json!(max_results);
    }
    if let Some(endpoint) = endpoint {
        input["endpoint"] = json!(endpoint);
    }
    if let Some(provider) = provider {
        input["provider"] = json!(provider);
    }
    if let Some(api_key) = api_key {
        input["api_key"] = json!(api_key);
    }
    Ok(input)
}

fn parse_web_extract_input(args: &[&str]) -> Result<serde_json::Value> {
    let mut url: Option<String> = None;
    let mut max_bytes: Option<usize> = None;
    let mut max_chars: Option<usize> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--max-bytes" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web extract <url> --max-bytes N"))?;
                max_bytes = Some(
                    value
                        .parse::<usize>()
                        .with_context(|| "--max-bytes must be a positive integer")?,
                );
                index += 2;
            }
            "--max-chars" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow!("usage: /web extract <url> --max-chars N"))?;
                max_chars = Some(
                    value
                        .parse::<usize>()
                        .with_context(|| "--max-chars must be a positive integer")?,
                );
                index += 2;
            }
            "help" | "--help" => {
                print_web_usage();
                index += 1;
            }
            value if url.is_none() => {
                url = Some(value.to_owned());
                index += 1;
            }
            value => {
                return Err(anyhow!("unexpected /web extract argument: {value}"));
            }
        }
    }
    let url = url.ok_or_else(|| anyhow!("usage: /web extract <url>"))?;
    let mut input = json!({ "url": url });
    if let Some(max_bytes) = max_bytes {
        input["max_bytes"] = json!(max_bytes);
    }
    if let Some(max_chars) = max_chars {
        input["max_chars"] = json!(max_chars);
    }
    Ok(input)
}

pub(super) fn print_web_usage() {
    println!(
        "web_usage: /web search <query> [--provider duckduckgo-html|brave|bing|serpapi|tavily] [--max-results N] [--endpoint URL] [--api-key KEY] | /web extract <url> [--max-bytes N] [--max-chars N]"
    );
}

fn emit_web_usage(runtime: &mut InteractiveChatRuntime) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        runtime.push_notice(WorkbenchNotice::info(
            "web",
            "usage is available in the fullscreen web cell",
        ));
    } else if runtime.default_inline_stdout() {
        print_default_inline_lines(web_usage_human_lines())?;
    } else {
        print_web_usage();
    }
    Ok(())
}

fn web_usage_human_lines() -> Vec<String> {
    vec![
        "• Web".to_owned(),
        "  usage: /web search <query> [--max-results N]".to_owned(),
        "  usage: /web extract <url> [--max-bytes N] [--max-chars N]".to_owned(),
    ]
}

fn handle_gateway_command(
    args: Vec<&str>,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    match args.as_slice() {
        [] | ["status"] => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(gateway_status_human_lines(paths)?)?;
            } else {
                print_gateway_status(paths)?;
            }
        }
        ["daemon", rest @ ..] => {
            run_gateway_daemon_workbench_command(rest, paths, workspace, Some(&runtime.agent.name))?
        }
        ["adapter", rest @ ..] => {
            run_gateway_adapter_workbench_command(rest, paths)?;
        }
        ["help"] | ["--help"] => print_gateway_usage(runtime)?,
        _ => print_gateway_usage(runtime)?,
    }
    Ok(())
}

fn print_gateway_usage(runtime: &InteractiveChatRuntime) -> Result<()> {
    if runtime.default_inline_stdout() {
        print_default_inline_lines(vec![
            "• Gateway".to_owned(),
            "  commands: /gateway status, /gateway daemon status".to_owned(),
            "  controls: /gateway daemon start, /gateway daemon stop, /gateway daemon restart"
                .to_owned(),
            "  adapters: /gateway adapter list, /gateway adapter enqueue".to_owned(),
        ])?;
    } else {
        println!(
            "gateway_usage: /gateway [status|daemon status|daemon start|daemon stop|daemon restart|adapter list|adapter enqueue|adapter render-delivery]"
        );
    }
    Ok(())
}

struct InteractiveMcpHttpCall {
    url: String,
    tool: String,
    arguments_json: String,
    max_response_bytes: usize,
}

struct InteractiveMcpStdioCall {
    command: String,
    tool: String,
    arguments_json: String,
    args: Vec<String>,
    timeout_ms: Option<u64>,
    max_output_bytes: Option<usize>,
}

async fn handle_mcp_command(
    args: Vec<&str>,
    ctx: &InteractiveCommandContext<'_>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    match args.as_slice() {
        [] | ["status"] => {
            if runtime.default_inline_stdout() {
                print_default_inline_lines(mcp_status_human_lines(ctx.config))?;
            } else {
                print_mcp_status(ctx.config);
            }
            append_workbench_evidence(runtime, "mcp", json!({"args": args}))?;
        }
        ["call-stdio", rest @ ..] => {
            let call = parse_mcp_call_stdio_args(rest)?;
            let tool = call.tool.clone();
            let mut input = json!({
                "command": call.command,
                "args": call.args,
                "tool": tool,
                "arguments": serde_json::from_str::<serde_json::Value>(&call.arguments_json)
                    .with_context(|| "invalid --arguments-json")?,
            });
            if let Some(timeout_ms) = call.timeout_ms {
                input["timeout_ms"] = json!(timeout_ms);
            }
            if let Some(max_output_bytes) = call.max_output_bytes {
                input["max_output_bytes"] = json!(max_output_bytes);
            }
            let result = runtime
                .session
                .execute_skill(ctx.registry, "mcp_stdio_call", input)
                .await?;
            println!(
                "mcp_stdio_call_json: {}",
                serde_json::to_string(&redact_json(result.output))?
            );
            append_workbench_evidence(runtime, "mcp_stdio_call", json!({"tool": tool}))?;
        }
        ["call-http", rest @ ..] => {
            let call = parse_mcp_call_http_args(rest)?;
            let report = run_mcp_http_call(
                &runtime.session,
                &call.url,
                &call.tool,
                &call.arguments_json,
                call.max_response_bytes,
            )
            .await?;
            println!("mcp_http_call_json: {}", serde_json::to_string(&report)?);
            append_workbench_evidence(
                runtime,
                "mcp_http_call",
                json!({
                    "url": redact_secrets(&call.url),
                    "tool": redact_secrets(&call.tool),
                    "max_response_bytes": call.max_response_bytes,
                    "network_egress": true,
                }),
            )?;
        }
        ["help"] | ["--help"] => emit_mcp_usage(runtime)?,
        _ => emit_mcp_usage(runtime)?,
    }
    Ok(())
}

fn parse_mcp_call_http_args(args: &[&str]) -> Result<InteractiveMcpHttpCall> {
    if args.len() < 2 {
        return Err(anyhow!(
            "usage: /mcp call-http <url> <tool> [--arguments-json JSON] [--max-response-bytes N]"
        ));
    }
    let mut call = InteractiveMcpHttpCall {
        url: args[0].to_owned(),
        tool: args[1].to_owned(),
        arguments_json: "{}".into(),
        max_response_bytes: 64 * 1024,
    };
    let mut index = 2;
    while index < args.len() {
        match args[index] {
            "--arguments-json" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-http <url> <tool> --arguments-json JSON")
                })?;
                call.arguments_json = (*value).to_owned();
                index += 2;
            }
            "--max-response-bytes" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-http <url> <tool> --max-response-bytes N")
                })?;
                call.max_response_bytes = value
                    .parse::<usize>()
                    .with_context(|| "--max-response-bytes must be a positive integer")?;
                index += 2;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /mcp call-http <url> <tool> [--arguments-json JSON] [--max-response-bytes N]"
                ));
            }
            value => {
                return Err(anyhow!(
                    "unknown /mcp call-http argument '{}'; expected --arguments-json or --max-response-bytes",
                    terminal_inline(value)
                ));
            }
        }
    }
    Ok(call)
}

fn parse_mcp_call_stdio_args(args: &[&str]) -> Result<InteractiveMcpStdioCall> {
    if args.len() < 2 {
        return Err(anyhow!(
            "usage: /mcp call-stdio <command> <tool> [--arguments-json JSON] [--args-json JSON_ARRAY] [--timeout-ms N] [--max-output-bytes N]"
        ));
    }
    let mut call = InteractiveMcpStdioCall {
        command: args[0].to_owned(),
        tool: args[1].to_owned(),
        arguments_json: "{}".into(),
        args: Vec::new(),
        timeout_ms: None,
        max_output_bytes: None,
    };
    let mut index = 2;
    while index < args.len() {
        match args[index] {
            "--arguments-json" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-stdio <command> <tool> --arguments-json JSON")
                })?;
                call.arguments_json = (*value).to_owned();
                index += 2;
            }
            "--args-json" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-stdio <command> <tool> --args-json JSON_ARRAY")
                })?;
                call.args = serde_json::from_str::<Vec<String>>(value)
                    .with_context(|| "--args-json must be a JSON string array")?;
                index += 2;
            }
            "--timeout-ms" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-stdio <command> <tool> --timeout-ms N")
                })?;
                call.timeout_ms = Some(
                    value
                        .parse::<u64>()
                        .with_context(|| "--timeout-ms must be a positive integer")?,
                );
                index += 2;
            }
            "--max-output-bytes" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow!("usage: /mcp call-stdio <command> <tool> --max-output-bytes N")
                })?;
                call.max_output_bytes = Some(
                    value
                        .parse::<usize>()
                        .with_context(|| "--max-output-bytes must be a positive integer")?,
                );
                index += 2;
            }
            "--help" | "help" => {
                return Err(anyhow!(
                    "usage: /mcp call-stdio <command> <tool> [--arguments-json JSON] [--args-json JSON_ARRAY]"
                ));
            }
            value => {
                return Err(anyhow!(
                    "unknown /mcp call-stdio argument '{}'",
                    terminal_inline(value)
                ));
            }
        }
    }
    Ok(call)
}

fn print_mcp_usage() {
    println!("usage: /mcp status");
    println!(
        "usage: /mcp call-stdio <command> <tool> [--arguments-json JSON] [--args-json JSON_ARRAY] [--timeout-ms N] [--max-output-bytes N]"
    );
    println!("usage: /mcp call-http <url> <tool> [--arguments-json JSON] [--max-response-bytes N]");
    println!(
        "mcp_policy: HTTP calls use NetworkEgress; stdio calls use harness ProcessRunner approval boundary"
    );
}

fn emit_mcp_usage(runtime: &InteractiveChatRuntime) -> Result<()> {
    if runtime.default_inline_stdout() {
        print_default_inline_lines(vec![
            "• MCP".to_owned(),
            "  status: /mcp status".to_owned(),
            "  stdio: /mcp call-stdio <command> <tool>".to_owned(),
            "  http: /mcp call-http <url> <tool>".to_owned(),
        ])?;
    } else {
        print_mcp_usage();
    }
    Ok(())
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

fn workbench_code_alias_command(command: &str, input: &str) -> Result<String> {
    let subcommand = command
        .strip_prefix('/')
        .ok_or_else(|| anyhow!("coding alias must start with '/'"))?;
    let rest = input
        .strip_prefix(command)
        .map(str::trim)
        .unwrap_or_default();
    if rest.is_empty() {
        Ok(subcommand.to_owned())
    } else {
        Ok(format!("{subcommand} {rest}"))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        parse_mcp_call_http_args, parse_timeline_request, parse_trace_request,
        workbench_code_alias_command,
    };

    #[test]
    fn parses_timeline_kind_page_and_turn_in_any_order() {
        let request = parse_timeline_request(vec!["--kind", "model", "--page", "2", "turn-one"])
            .expect("timeline request");

        assert_eq!(request.turn_filter.as_deref(), Some("turn-one"));
        assert_eq!(request.kind_filter.as_deref(), Some("model"));
        assert_eq!(request.page, 2);
    }

    #[test]
    fn parses_timeline_failed_and_approval_point_filters() {
        let failed = parse_timeline_request(vec!["turn-one", "--failed"]).expect("failed request");
        assert_eq!(failed.turn_filter.as_deref(), Some("turn-one"));
        assert_eq!(failed.point_filter.as_deref(), Some("failed"));

        let approval =
            parse_timeline_request(vec!["--approval", "--page", "3"]).expect("approval request");
        assert_eq!(approval.point_filter.as_deref(), Some("approval"));
        assert_eq!(approval.page, 3);
    }

    #[test]
    fn parses_mcp_call_http_arguments() {
        let call = parse_mcp_call_http_args(&[
            "http://127.0.0.1:9000/mcp",
            "search",
            "--arguments-json",
            r#"{"query":"ikaros"}"#,
            "--max-response-bytes",
            "128",
        ])
        .expect("mcp call args");

        assert_eq!(call.url, "http://127.0.0.1:9000/mcp");
        assert_eq!(call.tool, "search");
        assert_eq!(call.arguments_json, r#"{"query":"ikaros"}"#);
        assert_eq!(call.max_response_bytes, 128);
    }

    #[test]
    fn rejects_unknown_timeline_kind() {
        let error = parse_timeline_request(vec!["--kind", "missing"]).expect_err("timeline error");

        assert!(error.to_string().contains("unknown timeline kind"));
    }

    #[test]
    fn parses_trace_kind_turn_and_point_filters() {
        let request = parse_trace_request(vec!["turn-one", "--kind", "coding", "--failed"])
            .expect("trace request");

        assert_eq!(request.turn_filter.as_deref(), Some("turn-one"));
        assert_eq!(request.kind_filter.as_deref(), Some("coding"));
        assert_eq!(request.point_filter.as_deref(), Some("failed"));
    }

    #[test]
    fn workbench_code_aliases_delegate_to_code_subcommands() {
        assert_eq!(
            workbench_code_alias_command("/review", "/review --diff \"diff text\"")
                .expect("review alias"),
            "review --diff \"diff text\""
        );
        assert_eq!(
            workbench_code_alias_command(
                "/rollback",
                "/rollback coding-session --turn-id coding-turn"
            )
            .expect("rollback alias"),
            "rollback coding-session --turn-id coding-turn"
        );
    }
}
