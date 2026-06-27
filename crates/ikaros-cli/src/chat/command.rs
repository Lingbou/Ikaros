// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_agent::chat::ChatRunOptions;
use ikaros_agent::soul::load_or_default;
use ikaros_core::IkarosPaths;
use ikaros_execution::harness::CancellationToken;
use ikaros_host::host_agent_context;
use ikaros_providers::model::ModelUsageLedger;
use ikaros_terminal::{
    BRACKETED_PASTE_START, MULTILINE_TERMINATOR, WorkbenchInputState,
    WorkbenchLineEditorRenderState, WorkbenchLineInputUi, WorkbenchTerminalInputSessionGuard,
    fullscreen_terminal_event_input_available, handle_workbench_input_control,
    print_inline_turn_separator, read_bracketed_paste_message, read_multiline_message,
    read_workbench_terminal_line_input, terminal_inline, terminal_message,
};
use std::path::Path;

use super::args::ChatArgs;
use super::errors::print_interactive_command_error;
use super::history::{
    clear_chat_history, delete_chat_history_session, print_chat_history, print_chat_sessions,
    search_chat_history,
};
use super::interactive::{InteractiveCommandContext, handle_interactive_chat_command};
use super::notice::{WorkbenchNotice, WorkbenchNoticeKind};
use super::output::print_chat_message_result;
use super::pending::{drain_pending_interactive_inputs, requeue_failed_interactive_input};
use super::runtime::{initial_interactive_runtime, install_chat_cancellation_signal};
use super::session_id::interactive_chat_session_id;
use super::single::run_single_chat_message;
use super::slash::{
    queue_run_requested, slash_command_runs_pending_inputs_after_success,
    slash_command_separates_inline_output,
};
use super::turn::{InteractiveChatTurnContext, run_and_print_interactive_chat_turn_or_continue};
use super::workbench::{append_workbench_history, load_workbench_history_entries};
use super::{attachments, workbench};

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
        let result =
            run_single_chat_message(message, paths, workspace, agent_override, options).await?;
        print_chat_message_result(&result)?;
        return Ok(());
    }

    paths.ensure()?;
    let host = host_agent_context(paths, workspace, agent_override)?;
    let config = &host.config;
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
    let chat_session_id =
        interactive_chat_session_id(&host.agent_instance, options.session_id.as_deref())?;
    options.session_id = Some(chat_session_id.clone());
    let (mut runtime, registry) =
        initial_interactive_runtime(paths, workspace, config, agent_override, chat_session_id)?;
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
        let budget = workbench::active_model_budget_status(paths, &runtime)?;
        println!(
            "model_budget: {}",
            workbench::format_model_budget_status(&budget)
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
        config,
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
                    MULTILINE_TERMINATOR
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
            continue;
        }
        if input.starts_with('/') {
            push_slash_command_transcript(&mut runtime, input);
            if queue_run_requested(input) {
                drain_pending_interactive_inputs(&turn_context, &mut runtime, &options).await?;
                continue;
            }
            let command_context = InteractiveCommandContext {
                config,
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
                        terminal_inline(input.split_whitespace().next().unwrap_or(input)),
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
    }
    Ok(())
}

fn push_slash_command_transcript(
    runtime: &mut super::interactive::InteractiveChatRuntime,
    input: &str,
) {
    runtime.push_notice(WorkbenchNotice::info(
        "command input",
        &format!(
            "class={} action={} command={}",
            slash_command_transcript_class(input),
            slash_command_transcript_action(input),
            terminal_inline(input),
        ),
    ));
}

pub(in crate::chat) fn sanitize_interactive_input(input: &str) -> String {
    terminal_message(input.trim())
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
