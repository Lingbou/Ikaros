// SPDX-License-Identifier: GPL-3.0-only

mod attachments;
mod errors;
mod history;
mod interactive;
mod live;
mod notice;
mod output;
mod pending;
mod progress;
mod runtime;
mod screen;
mod slash;
mod terminal;
mod tui;
mod turn;
mod workbench;

use crate::resolve_agent_instance;
use anyhow::Result;
use clap::Args;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_harness::CancellationToken;
use ikaros_models::ModelUsageLedger;
use ikaros_runtime::{
    ChatRunOptions, DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET, chat_history_records_from_session_replay,
    new_chat_session_id, run_chat_message,
};
use ikaros_session::{SessionSource, SessionStore, SqliteSessionStore};
use ikaros_soul::load_or_default;
use std::path::Path;

use history::{
    clear_chat_history, delete_chat_history_session, print_chat_history, print_chat_sessions,
    search_chat_history,
};
use interactive::{
    InteractiveChatRuntime, InteractiveCommandContext, handle_interactive_chat_command,
};

use errors::print_interactive_command_error;
pub(in crate::chat) use errors::{
    interactive_chat_turn_error_actions, interactive_chat_turn_error_kind, suggested_budget_command,
};
#[cfg(test)]
use errors::{
    interactive_chat_turn_error_json_line, interactive_chat_turn_recovery_hint,
    interactive_command_error_json_line,
};
#[cfg(test)]
use live::{compact_live_event_cells, default_live_cell_event, live_cells_json_line};
use notice::{WorkbenchNotice, WorkbenchNoticeKind};
use output::print_chat_message_result;
pub(crate) use output::render_terminal_markdown;
use pending::{drain_pending_interactive_inputs, requeue_failed_interactive_input};
use runtime::{initial_interactive_runtime, install_chat_cancellation_signal};
use screen::refresh_persistent_workbench_screen;
use slash::{
    queue_run_requested, slash_command_refreshes_screen,
    slash_command_runs_pending_inputs_after_success, slash_command_separates_inline_output,
};
use terminal::{
    BRACKETED_PASTE_START, WorkbenchLineEditorRenderState, WorkbenchLineInputUi,
    WorkbenchTerminalInputSessionGuard, fullscreen_terminal_event_input_available,
    handle_workbench_input_control, print_inline_turn_separator, read_bracketed_paste_message,
    read_multiline_message, read_workbench_terminal_line_input,
};
use turn::{InteractiveChatTurnContext, run_and_print_interactive_chat_turn_or_continue};
use workbench::{WorkbenchInputState, append_workbench_history, load_workbench_history_entries};

#[derive(Debug, Args, Clone)]
pub(crate) struct ChatArgs {
    #[arg(long)]
    message: Option<String>,
    #[arg(long = "chat-session")]
    chat_session: Option<String>,
    #[arg(long)]
    stream: bool,
    #[arg(long = "image", value_name = "URL_OR_PATH")]
    image: Vec<String>,
    #[arg(long = "audio", value_name = "URL_OR_PATH")]
    audio: Vec<String>,
    #[arg(long = "file", value_name = "URL_OR_PATH")]
    file: Vec<String>,
    #[arg(long = "no-agent-loop")]
    no_agent_loop: bool,
    #[arg(long, default_value_t = 3)]
    memory_limit: usize,
    #[arg(long = "memory-search-limit", default_value_t = 0)]
    memory_search_limit: usize,
    #[arg(long, default_value_t = 0)]
    rag_top_k: usize,
    #[arg(long, default_value_t = 3)]
    history_context_limit: usize,
    #[arg(long, default_value_t = 12)]
    history_summary_limit: usize,
    #[arg(long = "context-token-budget", default_value_t = DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET)]
    context_token_budget: usize,
    #[arg(long = "context-engine")]
    context_engine: Option<String>,
    #[arg(long = "no-relationship-learning")]
    no_relationship_learning: bool,
    #[arg(long)]
    scope: Option<String>,
    #[arg(long)]
    no_context: bool,
    #[arg(long)]
    history: bool,
    #[arg(long)]
    sessions: bool,
    #[arg(long, default_value_t = 20)]
    history_limit: usize,
    #[arg(long = "history-session")]
    history_session: Option<String>,
    #[arg(long = "history-search")]
    history_search: Option<String>,
    #[arg(long = "history-delete-session")]
    history_delete_session: Option<String>,
    #[arg(long = "history-clear")]
    history_clear: bool,
}

impl Default for ChatArgs {
    fn default() -> Self {
        Self {
            message: None,
            chat_session: None,
            stream: false,
            image: Vec::new(),
            audio: Vec::new(),
            file: Vec::new(),
            no_agent_loop: false,
            memory_limit: 3,
            memory_search_limit: 0,
            rag_top_k: 0,
            history_context_limit: 3,
            history_summary_limit: 12,
            context_token_budget: DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET,
            context_engine: None,
            no_relationship_learning: false,
            scope: None,
            no_context: false,
            history: false,
            sessions: false,
            history_limit: 20,
            history_session: None,
            history_search: None,
            history_delete_session: None,
            history_clear: false,
        }
    }
}

pub(crate) async fn chat_command(
    args: ChatArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    chat_command_inner(args, paths, workspace, agent_override, true).await
}

pub(crate) async fn default_chat_command(
    args: ChatArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    chat_command_inner(args, paths, workspace, agent_override, false).await
}

async fn chat_command_inner(
    args: ChatArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    startup_diagnostics: bool,
) -> Result<()> {
    if args.history_clear {
        clear_chat_history(paths, workspace, agent_override)?;
        return Ok(());
    }
    if let Some(session_id) = args.history_delete_session.as_deref() {
        delete_chat_history_session(paths, workspace, agent_override, session_id)?;
        return Ok(());
    }
    if let Some(query) = args.history_search.as_deref() {
        search_chat_history(
            paths,
            workspace,
            agent_override,
            query,
            args.history_limit,
            args.history_session.as_deref(),
        )?;
        return Ok(());
    }
    if args.sessions {
        print_chat_sessions(paths, workspace, agent_override, args.history_limit)?;
        return Ok(());
    }
    if args.history {
        print_chat_history(
            paths,
            workspace,
            agent_override,
            args.history_limit,
            args.history_session.as_deref(),
        )?;
        return Ok(());
    }

    if let Some(message) = args.message.as_deref() {
        let mut options = ChatRunOptions::from(&args);
        options.content_blocks = attachments::content_blocks_from_args_resolving_paths(
            &args.image,
            &args.audio,
            &args.file,
            workspace,
        )?;
        let result = run_chat_message(message, paths, workspace, agent_override, options).await?;
        print_chat_message_result(&result)?;
        return Ok(());
    }

    paths.ensure()?;
    let config = IkarosConfig::load(&paths.config)?;
    let persona = load_or_default(&paths.persona_dir)?;
    let mut options = ChatRunOptions::from(&args);
    options.content_blocks = attachments::content_blocks_from_args_resolving_paths(
        &args.image,
        &args.audio,
        &args.file,
        workspace,
    )?;
    options.stream = true;
    options.cancellation = CancellationToken::new();
    install_chat_cancellation_signal(options.cancellation.clone());
    let chat_session_id = interactive_chat_session_id(
        &config,
        paths,
        workspace,
        agent_override,
        options.session_id.as_deref(),
    )?;
    options.session_id = Some(chat_session_id.clone());
    let (mut runtime, registry) =
        initial_interactive_runtime(paths, workspace, &config, agent_override, chat_session_id)?;
    options.session_state_db = Some(runtime.state_dir.join("state.db"));
    let show_startup_diagnostics = startup_diagnostics;
    runtime.pending_content_blocks = std::mem::take(&mut options.content_blocks);
    if show_startup_diagnostics && !runtime.pending_content_blocks.is_empty() {
        println!(
            "attachments_pending: {} attachments_force_single_call=true",
            runtime.pending_content_blocks.len()
        );
    }
    let usage_ledger = ModelUsageLedger::new(&paths.audit_dir);
    let refresh_inline_workbench = false;
    let default_line_input_ui = !show_startup_diagnostics;
    runtime.default_inline_ui = default_line_input_ui;

    if show_startup_diagnostics {
        println!(
            "Ikaros chat using provider={} persona={} agent={} mode={} stream={}. Type /help for commands.",
            runtime.provider.name(),
            persona.identity.name,
            runtime.agent.name,
            runtime.agent.mode(),
            options.stream
        );
        println!("audit: {}", runtime.session.audit.path().display());
        println!("model_usage: {}", usage_ledger.path().display());
        println!(
            "model_budget: {}",
            workbench::format_model_budget_status(&runtime.model_config, &usage_ledger)?
        );
        println!(
            "session_state_db: {}",
            runtime.state_dir.join("state.db").display()
        );
        println!("chat_timeline: session_store");
        println!(
            "workbench_history: {}",
            workbench::path_display(&workbench::workbench_history_path(paths))
        );
    }
    let mut line = String::new();
    let mut input_state =
        WorkbenchInputState::from_history(load_workbench_history_entries(paths, 200)?);
    let mut line_input_render_state = WorkbenchLineEditorRenderState::default();
    let mut line_input_intro_pending = default_line_input_ui;
    let line_input_terminal_modes =
        if default_line_input_ui && fullscreen_terminal_event_input_available() {
            WorkbenchTerminalInputSessionGuard::enable().ok()
        } else {
            None
        };
    let line_input_terminal_modes_enabled = line_input_terminal_modes.is_some();
    let turn_context = InteractiveChatTurnContext {
        config: &config,
        paths,
        persona: &persona,
        registry: &registry,
    };
    loop {
        let line_input_ui = default_line_input_ui.then(|| {
            let show_intro = std::mem::take(&mut line_input_intro_pending);
            WorkbenchLineInputUi::new(
                runtime.model_config.model.clone(),
                workbench::path_display(workspace),
                show_intro,
            )
        });
        let Some(raw_input) = read_workbench_terminal_line_input(
            &mut input_state,
            &mut line,
            line_input_ui.as_ref(),
            line_input_terminal_modes_enabled,
            &mut line_input_render_state,
        )?
        else {
            break;
        };
        if handle_workbench_input_control(
            &raw_input,
            &mut input_state,
            runtime.default_inline_stdout(),
        ) {
            continue;
        }
        let raw_input = raw_input.trim();
        if raw_input.starts_with(BRACKETED_PASTE_START) {
            let Some(message) = read_bracketed_paste_message(raw_input)? else {
                break;
            };
            let message = message.trim();
            if message.is_empty() {
                continue;
            }
            if !runtime.machine_stdout_quiet() {
                println!("bracketed_paste: accepted");
            }
            runtime.push_notice(WorkbenchNotice::info(
                "paste",
                "accepted bracketed paste input",
            ));
            append_workbench_history(paths, message)?;
            input_state.record_history(message);
            if run_and_print_interactive_chat_turn_or_continue(
                message,
                &turn_context,
                &mut runtime,
                &options,
            )
            .await?
            {
                drain_pending_interactive_inputs(&turn_context, &mut runtime, &options).await?;
            } else {
                requeue_failed_interactive_input(&mut runtime, message, "bracketed_paste");
            }
            refresh_visible_workbench_screen(
                &config,
                paths,
                workspace,
                &runtime,
                &options,
                &usage_ledger,
                refresh_inline_workbench,
            )?;
            continue;
        }
        let input = sanitize_interactive_input(raw_input);
        let input = input.as_str();
        if input.eq_ignore_ascii_case("/quit") || input.eq_ignore_ascii_case("/exit") {
            break;
        }
        if input.eq_ignore_ascii_case("/multi") {
            if !runtime.machine_stdout_quiet() {
                println!(
                    "multiline: end with a single '{}' line",
                    workbench::MULTILINE_TERMINATOR
                );
            }
            runtime.push_notice(WorkbenchNotice::info(
                "multiline",
                "collecting multiline input until terminator",
            ));
            let Some(message) = read_multiline_message()? else {
                break;
            };
            let message = message.trim();
            if message.is_empty() {
                continue;
            }
            append_workbench_history(paths, message)?;
            input_state.record_history(message);
            if run_and_print_interactive_chat_turn_or_continue(
                message,
                &turn_context,
                &mut runtime,
                &options,
            )
            .await?
            {
                drain_pending_interactive_inputs(&turn_context, &mut runtime, &options).await?;
            } else {
                requeue_failed_interactive_input(&mut runtime, message, "multiline");
            }
            refresh_visible_workbench_screen(
                &config,
                paths,
                workspace,
                &runtime,
                &options,
                &usage_ledger,
                refresh_inline_workbench,
            )?;
            continue;
        }
        if input.starts_with('/') {
            push_slash_command_transcript(&mut runtime, input);
            let refresh_after_command = slash_command_refreshes_screen(input);
            if queue_run_requested(input) {
                drain_pending_interactive_inputs(&turn_context, &mut runtime, &options).await?;
                refresh_visible_workbench_screen(
                    &config,
                    paths,
                    workspace,
                    &runtime,
                    &options,
                    &usage_ledger,
                    refresh_inline_workbench,
                )?;
                continue;
            }
            let command_context = InteractiveCommandContext {
                config: &config,
                paths,
                workspace,
                usage_ledger: &usage_ledger,
                registry: &registry,
            };
            if let Err(error) =
                handle_interactive_chat_command(input, &command_context, &mut runtime, &mut options)
                    .await
            {
                print_interactive_command_error(&runtime, input, &error);
                if runtime.default_inline_stdout() && slash_command_separates_inline_output(input) {
                    print_inline_turn_separator();
                }
                runtime.push_notice(WorkbenchNotice::error(
                    "command failed",
                    &format!("command={} error={}", input, error),
                ));
                let screen_command_printed_inline = input.split_whitespace().next()
                    == Some("/screen")
                    && !runtime.fullscreen_stdout_quiet();
                if (refresh_after_command && !screen_command_printed_inline)
                    || runtime.fullscreen_stdout_quiet()
                {
                    refresh_visible_workbench_screen(
                        &config,
                        paths,
                        workspace,
                        &runtime,
                        &options,
                        &usage_ledger,
                        refresh_inline_workbench,
                    )?;
                }
                continue;
            }
            if runtime.default_inline_stdout() && slash_command_separates_inline_output(input) {
                print_inline_turn_separator();
            }
            if !runtime.default_inline_stdout() {
                runtime.push_notice(WorkbenchNotice::info(
                    "command executed",
                    &format!("command={input}"),
                ));
            }
            if (runtime.take_pending_input_drain_request()
                || slash_command_runs_pending_inputs_after_success(input))
                && !runtime.pending_inputs.is_empty()
            {
                if !runtime.machine_stdout_quiet() {
                    println!(
                        "pending_input_autorun: trigger={} pending_inputs={}",
                        workbench::terminal_inline(
                            input.split_whitespace().next().unwrap_or(input)
                        ),
                        runtime.pending_inputs.len()
                    );
                }
                runtime.push_notice(WorkbenchNotice::new(
                    WorkbenchNoticeKind::Continuation,
                    "pending input",
                    "approval or budget action completed; draining queued input",
                ));
                drain_pending_interactive_inputs(&turn_context, &mut runtime, &options).await?;
            }
            let screen_command_printed_inline = input.split_whitespace().next() == Some("/screen")
                && !runtime.fullscreen_stdout_quiet();
            if (refresh_after_command && !screen_command_printed_inline)
                || runtime.fullscreen_stdout_quiet()
            {
                refresh_visible_workbench_screen(
                    &config,
                    paths,
                    workspace,
                    &runtime,
                    &options,
                    &usage_ledger,
                    refresh_inline_workbench,
                )?;
            }
            continue;
        }
        if input.is_empty() {
            continue;
        }
        append_workbench_history(paths, input)?;
        input_state.record_history(input);
        if run_and_print_interactive_chat_turn_or_continue(
            input,
            &turn_context,
            &mut runtime,
            &options,
        )
        .await?
        {
            drain_pending_interactive_inputs(&turn_context, &mut runtime, &options).await?;
        } else {
            requeue_failed_interactive_input(&mut runtime, input, "interactive");
        }
        refresh_visible_workbench_screen(
            &config,
            paths,
            workspace,
            &runtime,
            &options,
            &usage_ledger,
            refresh_inline_workbench,
        )?;
    }
    Ok(())
}

fn push_slash_command_transcript(runtime: &mut interactive::InteractiveChatRuntime, input: &str) {
    runtime.push_notice(WorkbenchNotice::info(
        "command input",
        &format!(
            "class={} action={} command={}",
            slash_command_transcript_class(input),
            slash_command_transcript_action(input),
            workbench::terminal_inline(input),
        ),
    ));
}

fn sanitize_interactive_input(input: &str) -> String {
    workbench::terminal_message(input.trim())
}

fn slash_command_transcript_class(input: &str) -> &'static str {
    let mut parts = input.split_whitespace();
    match (parts.next().unwrap_or_default(), parts.next()) {
        ("/session", Some("resume" | "export")) => "command",
        ("/session" | "/sessions" | "/context" | "/memory" | "/mentions", _) => "inspect_context",
        ("/status" | "/model" | "/provider" | "/rag" | "/tools" | "/mcp" | "/api" | "/diff", _) => {
            "inspect"
        }
        ("/clear" | "/new", _) => "ui",
        ("/screen" | "/commands" | "/help", _) => "ui",
        _ => "command",
    }
}

fn slash_command_transcript_action(input: &str) -> &'static str {
    let mut parts = input.split_whitespace();
    match (parts.next().unwrap_or_default(), parts.next()) {
        ("/session", Some("resume")) => "resume_session",
        ("/session", Some("export")) => "export_session",
        ("/session", _) => "inspect_session",
        ("/sessions", _) => "inspect_sessions",
        ("/context", _) => "inspect_context",
        ("/memory", _) => "inspect_memory",
        ("/mentions", _) => "inspect_mentions",
        ("/status", _) => "inspect_status",
        ("/model", _) => "inspect_model",
        ("/provider", _) => "inspect_provider",
        ("/clear" | "/new", _) => "clear_session",
        ("/commands" | "/help", _) => "inspect_commands",
        ("/screen", _) => "open_ui",
        _ => "run_command",
    }
}

fn interactive_chat_session_id(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    explicit_session_id: Option<&str>,
) -> Result<String> {
    if let Some(session_id) = explicit_session_id {
        return Ok(session_id.to_owned());
    }
    Ok(
        recent_interactive_chat_session_id(config, paths, workspace, agent_override)?
            .unwrap_or_else(new_chat_session_id),
    )
}

fn recent_interactive_chat_session_id(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Option<String>> {
    let agent = resolve_agent_instance(config, agent_override, workspace, &paths.home)?;
    let store = SqliteSessionStore::new(&agent.state_dir);
    if !store.path().is_file() {
        return Ok(None);
    }
    let mut latest: Option<ikaros_session::SessionRecord> = None;
    for session in store.session_records()? {
        if session.ended_at.is_some()
            || !matches!(session.source, SessionSource::Cli)
            || session.agent_id.as_deref() != Some(agent.agent_id.as_str())
            || session.workspace.as_deref() != Some(agent.workspace.as_path())
        {
            continue;
        }
        let Some(replay) = store.replay_session(&session.session_id)? else {
            continue;
        };
        if chat_history_records_from_session_replay(&replay).is_empty() {
            continue;
        }
        if latest.as_ref().is_none_or(|current| {
            session
                .started_at
                .cmp(&current.started_at)
                .then_with(|| session.session_id.as_str().cmp(current.session_id.as_str()))
                .is_gt()
        }) {
            latest = Some(session);
        }
    }
    Ok(latest.map(|session| session.session_id.to_string()))
}

fn refresh_visible_workbench_screen(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
    refresh_inline_workbench: bool,
) -> Result<()> {
    if !refresh_inline_workbench {
        return Ok(());
    }
    refresh_persistent_workbench_screen(config, paths, workspace, runtime, options, usage_ledger)
}

impl From<&ChatArgs> for ChatRunOptions {
    fn from(args: &ChatArgs) -> Self {
        Self {
            stream: args.stream,
            agent_loop: !args.no_agent_loop,
            memory_limit: args.memory_limit,
            memory_search_limit: args.memory_search_limit,
            rag_top_k: args.rag_top_k,
            history_context_limit: args.history_context_limit,
            history_summary_limit: args.history_summary_limit,
            context_token_budget: args.context_token_budget,
            context_engine: args.context_engine.clone(),
            relationship_learning: !args.no_relationship_learning,
            scope: args.scope.clone(),
            no_context: args.no_context,
            session_id: args.chat_session.clone(),
            turn_id: None,
            session_source: None,
            session_state_db: None,
            safe_tools: false,
            content_blocks: attachments::content_blocks_from_args(
                &args.image,
                &args.audio,
                &args.file,
            ),
            cancellation: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests;
