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
mod startup;
mod terminal;
mod turn;
mod workbench;

use anyhow::Result;
use clap::Args;
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_harness::CancellationToken;
use ikaros_models::ModelUsageLedger;
use ikaros_runtime::{
    ChatRunOptions, DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET, new_chat_session_id, run_chat_message,
};
use ikaros_soul::load_or_default;
use std::path::Path;

use history::{
    clear_chat_history, delete_chat_history_session, print_chat_history, print_chat_sessions,
    search_chat_history,
};
use interactive::{InteractiveCommandContext, handle_interactive_chat_command};

use errors::print_workbench_command_error;
pub(in crate::chat) use errors::{
    interactive_chat_turn_error_actions, interactive_chat_turn_error_kind, suggested_budget_command,
};
#[cfg(test)]
use errors::{
    interactive_chat_turn_error_json_line, interactive_chat_turn_recovery_hint,
    workbench_command_error_json_line,
};
#[cfg(test)]
use live::{compact_live_event_cells, default_live_cell_event, live_cells_json_line};
use notice::{WorkbenchNotice, WorkbenchNoticeKind};
use output::print_chat_message_result;
pub(crate) use output::render_terminal_markdown;
use pending::{drain_pending_interactive_inputs, requeue_failed_interactive_input};
use runtime::{initial_interactive_runtime, install_chat_cancellation_signal};
use screen::{refresh_persistent_workbench_screen, sync_fullscreen_terminal_session};
use slash::{
    queue_run_requested, slash_command_refreshes_screen,
    slash_command_runs_pending_inputs_after_success,
};
use startup::WorkbenchStartupScreen;
use terminal::{
    BRACKETED_PASTE_START, fullscreen_terminal_event_input_available,
    handle_workbench_input_control, read_bracketed_paste_message, read_fullscreen_workbench_input,
    read_multiline_message, read_workbench_terminal_line_input,
};
#[cfg(test)]
use terminal::{FullscreenScreenAction, take_fullscreen_screen_action};
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
    chat_command_inner(
        args,
        paths,
        workspace,
        agent_override,
        WorkbenchStartupScreen::None,
    )
    .await
}

pub(crate) async fn workbench_command(
    args: ChatArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    chat_command_inner(
        args,
        paths,
        workspace,
        agent_override,
        WorkbenchStartupScreen::Inline,
    )
    .await
}

pub(crate) async fn tui_command(
    args: ChatArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    chat_command_inner(
        args,
        paths,
        workspace,
        agent_override,
        WorkbenchStartupScreen::Fullscreen,
    )
    .await
}

async fn chat_command_inner(
    args: ChatArgs,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    startup_screen: WorkbenchStartupScreen,
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
    let persona = load_or_default(&paths.persona)?;
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
    let chat_session_id = options
        .session_id
        .clone()
        .unwrap_or_else(new_chat_session_id);
    options.session_id = Some(chat_session_id.clone());
    let (mut runtime, registry) =
        initial_interactive_runtime(paths, workspace, &config, agent_override, chat_session_id)?;
    let requested_startup_screen = startup_screen;
    let startup_screen = if matches!(requested_startup_screen, WorkbenchStartupScreen::Fullscreen)
        && !fullscreen_terminal_event_input_available()
    {
        eprintln!("warning: fullscreen unavailable; using line input");
        WorkbenchStartupScreen::None
    } else {
        requested_startup_screen
    };
    let show_startup_diagnostics =
        !matches!(requested_startup_screen, WorkbenchStartupScreen::Fullscreen);
    runtime.pending_content_blocks = std::mem::take(&mut options.content_blocks);
    if show_startup_diagnostics && !runtime.pending_content_blocks.is_empty() {
        println!(
            "attachments_pending: {} attachments_force_single_call=true",
            runtime.pending_content_blocks.len()
        );
    }
    if matches!(startup_screen, WorkbenchStartupScreen::Fullscreen) {
        workbench::apply_workbench_screen_args(&mut runtime.screen_state, &["--fullscreen"])?;
        runtime.persistent_fullscreen = true;
    }
    let usage_ledger = ModelUsageLedger::new(&paths.audit_dir);

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
    let mut fullscreen_terminal = None;
    match startup_screen {
        WorkbenchStartupScreen::None => {}
        WorkbenchStartupScreen::Inline => {
            workbench::print_screen_status(
                &config,
                paths,
                workspace,
                &runtime,
                &options,
                &usage_ledger,
            )?;
        }
        WorkbenchStartupScreen::Fullscreen => {
            sync_fullscreen_terminal_session(&mut runtime, &mut fullscreen_terminal)?;
            refresh_persistent_workbench_screen(
                &config,
                paths,
                workspace,
                &runtime,
                &options,
                &usage_ledger,
                fullscreen_terminal.as_mut(),
            )?;
        }
    }
    let mut line = String::new();
    let mut input_state =
        WorkbenchInputState::from_history(load_workbench_history_entries(paths, 200)?);
    let turn_context = InteractiveChatTurnContext {
        config: &config,
        paths,
        workspace,
        persona: &persona,
        registry: &registry,
        usage_ledger: &usage_ledger,
    };
    loop {
        sync_fullscreen_terminal_session(&mut runtime, &mut fullscreen_terminal)?;
        let fullscreen_input = runtime.persistent_fullscreen && runtime.screen_state.fullscreen();
        let raw_input = if fullscreen_input {
            let Some(input) = read_fullscreen_workbench_input(
                &config,
                paths,
                workspace,
                &mut runtime,
                &options,
                &usage_ledger,
                &mut input_state,
                fullscreen_terminal.as_mut(),
            )?
            else {
                break;
            };
            input
        } else {
            let Some(input) = read_workbench_terminal_line_input(&mut input_state, &mut line)?
            else {
                break;
            };
            input
        };
        if !fullscreen_input && handle_workbench_input_control(&raw_input, &mut input_state) {
            continue;
        }
        let input = raw_input.trim();
        if input.eq_ignore_ascii_case("/quit") || input.eq_ignore_ascii_case("/exit") {
            break;
        }
        if input.starts_with(BRACKETED_PASTE_START) {
            let Some(message) = read_bracketed_paste_message(input)? else {
                break;
            };
            let message = message.trim();
            if message.is_empty() {
                continue;
            }
            if !runtime.fullscreen_stdout_quiet() {
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
                fullscreen_terminal.as_mut(),
            )
            .await?
            {
                drain_pending_interactive_inputs(
                    &turn_context,
                    &mut runtime,
                    &options,
                    fullscreen_terminal.as_mut(),
                )
                .await?;
            } else {
                requeue_failed_interactive_input(&mut runtime, message, "bracketed_paste");
            }
            refresh_persistent_workbench_screen(
                &config,
                paths,
                workspace,
                &runtime,
                &options,
                &usage_ledger,
                fullscreen_terminal.as_mut(),
            )?;
            continue;
        }
        if input.eq_ignore_ascii_case("/multi") {
            if !runtime.fullscreen_stdout_quiet() {
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
                fullscreen_terminal.as_mut(),
            )
            .await?
            {
                drain_pending_interactive_inputs(
                    &turn_context,
                    &mut runtime,
                    &options,
                    fullscreen_terminal.as_mut(),
                )
                .await?;
            } else {
                requeue_failed_interactive_input(&mut runtime, message, "multiline");
            }
            refresh_persistent_workbench_screen(
                &config,
                paths,
                workspace,
                &runtime,
                &options,
                &usage_ledger,
                fullscreen_terminal.as_mut(),
            )?;
            continue;
        }
        if input.starts_with('/') {
            let refresh_after_command = slash_command_refreshes_screen(input);
            if queue_run_requested(input) {
                drain_pending_interactive_inputs(
                    &turn_context,
                    &mut runtime,
                    &options,
                    fullscreen_terminal.as_mut(),
                )
                .await?;
                refresh_persistent_workbench_screen(
                    &config,
                    paths,
                    workspace,
                    &runtime,
                    &options,
                    &usage_ledger,
                    fullscreen_terminal.as_mut(),
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
                print_workbench_command_error(&runtime, input, &error);
                runtime.push_notice(WorkbenchNotice::error(
                    "command failed",
                    &format!("command={} error={}", input, error),
                ));
                if refresh_after_command || runtime.fullscreen_stdout_quiet() {
                    refresh_persistent_workbench_screen(
                        &config,
                        paths,
                        workspace,
                        &runtime,
                        &options,
                        &usage_ledger,
                        fullscreen_terminal.as_mut(),
                    )?;
                }
                continue;
            }
            runtime.push_notice(WorkbenchNotice::info(
                "command executed",
                &format!("command={input}"),
            ));
            if (runtime.take_pending_input_drain_request()
                || slash_command_runs_pending_inputs_after_success(input))
                && !runtime.pending_inputs.is_empty()
            {
                if !runtime.fullscreen_stdout_quiet() {
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
                drain_pending_interactive_inputs(
                    &turn_context,
                    &mut runtime,
                    &options,
                    fullscreen_terminal.as_mut(),
                )
                .await?;
            }
            if refresh_after_command || runtime.fullscreen_stdout_quiet() {
                refresh_persistent_workbench_screen(
                    &config,
                    paths,
                    workspace,
                    &runtime,
                    &options,
                    &usage_ledger,
                    fullscreen_terminal.as_mut(),
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
            fullscreen_terminal.as_mut(),
        )
        .await?
        {
            drain_pending_interactive_inputs(
                &turn_context,
                &mut runtime,
                &options,
                fullscreen_terminal.as_mut(),
            )
            .await?;
        } else {
            requeue_failed_interactive_input(&mut runtime, input, "interactive");
        }
        refresh_persistent_workbench_screen(
            &config,
            paths,
            workspace,
            &runtime,
            &options,
            &usage_ledger,
            fullscreen_terminal.as_mut(),
        )?;
    }
    Ok(())
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
