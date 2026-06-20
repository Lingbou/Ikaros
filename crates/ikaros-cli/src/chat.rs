// SPDX-License-Identifier: GPL-3.0-only

mod interactive;
mod output;
mod workbench;

use crate::{resolve_agent_instance, session_and_registry_for_instance};
use anyhow::Result;
use clap::Args;
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile, redact_secrets};
use ikaros_harness::{CancellationToken, SkillRegistry};
use ikaros_models::{
    ModelProvider, ModelUsageLedger, governed_provider_from_config_with_http_client,
};
use ikaros_runtime::{
    ChatHistoryRecord, ChatHistorySessionSummary, ChatHistoryStore, ChatRunOptions,
    ChatTurnEventOptions, ChatTurnReport, DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET, EgressModelHttpClient,
    new_chat_session_id, run_chat_message, run_chat_turn_with_events,
};
use ikaros_session::{
    PersistingAgentTurnSink, SessionId, SessionSource, SessionStore, SqliteSessionStore, TurnId,
};
use ikaros_soul::load_or_default;
use std::{
    collections::VecDeque,
    io::{self, Write},
    path::Path,
    sync::Arc,
};

use interactive::{InteractiveChatRuntime, handle_interactive_chat_command};
use output::{print_chat_content, print_chat_message_result};
use workbench::{MULTILINE_TERMINATOR, append_workbench_history};

const BRACKETED_PASTE_START: &str = "\u{1b}[200~";
const BRACKETED_PASTE_END: &str = "\u{1b}[201~";

#[derive(Debug, Args, Clone)]
pub(crate) struct ChatArgs {
    #[arg(long)]
    message: Option<String>,
    #[arg(long = "chat-session")]
    chat_session: Option<String>,
    #[arg(long)]
    stream: bool,
    #[arg(long = "no-agent-loop")]
    no_agent_loop: bool,
    #[arg(long, default_value_t = 3)]
    memory_limit: usize,
    #[arg(long, default_value_t = 0)]
    rag_top_k: usize,
    #[arg(long, default_value_t = 3)]
    history_context_limit: usize,
    #[arg(long, default_value_t = 12)]
    history_summary_limit: usize,
    #[arg(long = "context-token-budget", default_value_t = DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET)]
    context_token_budget: usize,
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
            no_agent_loop: false,
            memory_limit: 3,
            rag_top_k: 0,
            history_context_limit: 3,
            history_summary_limit: 12,
            context_token_budget: DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET,
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
    if args.history_clear {
        clear_chat_history(paths)?;
        return Ok(());
    }
    if let Some(session_id) = args.history_delete_session.as_deref() {
        delete_chat_history_session(paths, session_id)?;
        return Ok(());
    }
    if let Some(query) = args.history_search.as_deref() {
        search_chat_history(
            paths,
            query,
            args.history_limit,
            args.history_session.as_deref(),
        )?;
        return Ok(());
    }
    if args.sessions {
        print_chat_sessions(paths, args.history_limit)?;
        return Ok(());
    }
    if args.history {
        print_chat_history(paths, args.history_limit, args.history_session.as_deref())?;
        return Ok(());
    }

    if let Some(message) = args.message.as_deref() {
        let result = run_chat_message(
            message,
            paths,
            workspace,
            agent_override,
            ChatRunOptions::from(&args),
        )
        .await?;
        print_chat_message_result(&result)?;
        return Ok(());
    }

    paths.ensure()?;
    let config = IkarosConfig::load(&paths.config)?;
    let persona = load_or_default(&paths.persona)?;
    let history_store =
        ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let mut options = ChatRunOptions::from(&args);
    options.stream = true;
    options.cancellation = CancellationToken::new();
    install_chat_cancellation_signal(options.cancellation.clone());
    let chat_session_id = options
        .session_id
        .clone()
        .unwrap_or_else(new_chat_session_id);
    options.session_id = Some(chat_session_id.clone());
    options.chat_history_path = Some(history_store.path().to_path_buf());
    options.chat_history_backend = Some(history_store.backend_name().into());
    let (mut runtime, registry) =
        initial_interactive_runtime(paths, workspace, &config, agent_override, chat_session_id)?;
    let provider = governed_provider_from_config_with_http_client(
        &config.model.default,
        &config.providers.model,
        &paths.audit_dir,
        Some(std::sync::Arc::new(EgressModelHttpClient::new(
            runtime.session.env.clone(),
        ))),
    )?;
    let usage_ledger = ModelUsageLedger::new(&paths.audit_dir);

    println!(
        "Ikaros chat using provider={} persona={} agent={} mode={} stream={}. Type /help for commands.",
        provider.name(),
        persona.identity.name,
        runtime.agent.name,
        runtime.agent.mode(),
        options.stream
    );
    println!("audit: {}", runtime.session.audit.path().display());
    println!("model_usage: {}", usage_ledger.path().display());
    println!("chat_history: {}", history_store.path().display());
    println!("chat_history_backend: {}", history_store.backend_name());
    println!(
        "workbench_history: {}",
        workbench::path_display(&workbench::workbench_history_path(paths))
    );
    let mut line = String::new();
    loop {
        print!("ikaros> ");
        io::stdout().flush()?;
        line.clear();
        if io::stdin().read_line(&mut line)? == 0 {
            break;
        }
        let input = line.trim();
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
            println!("bracketed_paste: accepted");
            append_workbench_history(paths, message)?;
            run_and_print_interactive_chat_turn(
                message,
                &persona,
                provider.as_ref(),
                &mut runtime,
                &registry,
                &options,
            )
            .await?;
            drain_pending_interactive_inputs(
                paths,
                &persona,
                provider.as_ref(),
                &mut runtime,
                &registry,
                &options,
            )
            .await?;
            continue;
        }
        if input.eq_ignore_ascii_case("/multi") {
            println!(
                "multiline: end with a single '{}' line",
                MULTILINE_TERMINATOR
            );
            let Some(message) = read_multiline_message()? else {
                break;
            };
            let message = message.trim();
            if message.is_empty() {
                continue;
            }
            append_workbench_history(paths, message)?;
            run_and_print_interactive_chat_turn(
                message,
                &persona,
                provider.as_ref(),
                &mut runtime,
                &registry,
                &options,
            )
            .await?;
            drain_pending_interactive_inputs(
                paths,
                &persona,
                provider.as_ref(),
                &mut runtime,
                &registry,
                &options,
            )
            .await?;
            continue;
        }
        if input.starts_with('/') {
            handle_interactive_chat_command(
                input,
                &config,
                paths,
                workspace,
                &mut runtime,
                &mut options,
                &usage_ledger,
            )
            .await?;
            continue;
        }
        if input.is_empty() {
            continue;
        }
        append_workbench_history(paths, input)?;
        run_and_print_interactive_chat_turn(
            input,
            &persona,
            provider.as_ref(),
            &mut runtime,
            &registry,
            &options,
        )
        .await?;
        drain_pending_interactive_inputs(
            paths,
            &persona,
            provider.as_ref(),
            &mut runtime,
            &registry,
            &options,
        )
        .await?;
    }
    Ok(())
}

async fn run_and_print_interactive_chat_turn(
    input: &str,
    persona: &ikaros_soul::PersonaProfile,
    provider: &dyn ModelProvider,
    runtime: &mut InteractiveChatRuntime,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<ChatTurnReport> {
    println!(
        "chat_turn: started session={} stream={} agent_loop={}",
        redact_secrets(&runtime.chat_session_id),
        options.stream,
        options.agent_loop
    );
    let report =
        run_interactive_chat_turn(input, persona, provider, runtime, registry, options).await?;
    print_interactive_chat_report(&report)?;
    println!(
        "chat_turn: completed session={} streamed={} stream_chunks={}",
        redact_secrets(&runtime.chat_session_id),
        report.streamed,
        report.stream_chunks.len()
    );
    Ok(report)
}

async fn drain_pending_interactive_inputs(
    paths: &IkarosPaths,
    persona: &ikaros_soul::PersonaProfile,
    provider: &dyn ModelProvider,
    runtime: &mut InteractiveChatRuntime,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<()> {
    let total = runtime.pending_inputs.len();
    if total == 0 {
        return Ok(());
    }
    let mut pending = VecDeque::new();
    std::mem::swap(&mut pending, &mut runtime.pending_inputs);
    for (index, input) in pending.into_iter().enumerate() {
        println!("pending_input: running index={} total={}", index + 1, total);
        println!("pending_input_message: {}", redact_secrets(&input));
        append_workbench_history(paths, &input)?;
        run_and_print_interactive_chat_turn(&input, persona, provider, runtime, registry, options)
            .await?;
    }
    Ok(())
}

async fn run_interactive_chat_turn(
    input: &str,
    persona: &ikaros_soul::PersonaProfile,
    provider: &dyn ModelProvider,
    runtime: &mut InteractiveChatRuntime,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<ChatTurnReport> {
    let chat_session_id = options
        .session_id
        .clone()
        .unwrap_or_else(|| runtime.chat_session_id.clone());
    let session_id = SessionId::from(chat_session_id);
    let turn_id = TurnId::new();
    let session_store: Arc<dyn SessionStore> =
        Arc::new(SqliteSessionStore::new(&runtime.state_dir));
    let parent_entry_id = session_store
        .get_session(&session_id)?
        .and_then(|session| session.active_leaf_entry_id);
    let event_sink = PersistingAgentTurnSink::new(session_store)
        .with_source(options.session_source.clone().unwrap_or(SessionSource::Cli))
        .with_agent_id(runtime.agent_id.clone())
        .with_workspace(runtime.workspace.clone());
    let report = match run_chat_turn_with_events(
        input,
        persona,
        provider,
        &runtime.agent,
        &runtime.session,
        registry,
        ChatTurnEventOptions {
            options,
            event_sink: &event_sink,
            session_sink: Some(&event_sink),
            parent_entry_id,
            turn_id: Some(turn_id),
        },
    )
    .await
    {
        Ok(report) => report,
        Err(error) => {
            if event_sink.commit().is_err() {
                let _ = event_sink.rollback();
            }
            return Err(error.into());
        }
    };
    event_sink.commit()?;
    Ok(report)
}

fn print_interactive_chat_report(report: &ChatTurnReport) -> Result<()> {
    println!(
        "context: relationship={} references={} history={} memory={} rag={} relationship_candidates_created={}",
        report.relationship_hits,
        report.reference_hits,
        report.history_hits,
        report.memory_hits,
        report.rag_hits,
        report.relationship_candidates_created
    );
    if let Some(path) = &report.chat_history_path {
        println!("chat_history: {}", path.display());
    }
    if report.streamed {
        println!("chat_stream: start");
        println!("stream_chunks: {}", report.stream_chunks.len());
    }
    print_chat_content(report)?;
    if report.streamed {
        println!("chat_stream: done");
    }
    Ok(())
}

fn read_multiline_message() -> Result<Option<String>> {
    let mut message = String::new();
    let mut line = String::new();
    loop {
        print!("... ");
        io::stdout().flush()?;
        line.clear();
        if io::stdin().read_line(&mut line)? == 0 {
            return if message.is_empty() {
                Ok(None)
            } else {
                Ok(Some(message))
            };
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == MULTILINE_TERMINATOR {
            return Ok(Some(message));
        }
        message.push_str(trimmed);
        message.push('\n');
    }
}

fn read_bracketed_paste_message(first_line: &str) -> Result<Option<String>> {
    let mut message = String::new();
    let mut current = first_line
        .strip_prefix(BRACKETED_PASTE_START)
        .unwrap_or(first_line)
        .to_owned();
    loop {
        if let Some((before_end, _after_end)) = current.split_once(BRACKETED_PASTE_END) {
            message.push_str(before_end.trim_end_matches(['\n', '\r']));
            return Ok(Some(message));
        }
        message.push_str(current.trim_end_matches(['\n', '\r']));
        message.push('\n');
        current.clear();
        if io::stdin().read_line(&mut current)? == 0 {
            return if message.is_empty() {
                Ok(None)
            } else {
                Ok(Some(message))
            };
        }
    }
}

fn install_chat_cancellation_signal(cancellation: CancellationToken) {
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancellation.cancel();
            eprintln!("chat_cancel_requested: waiting for the running provider/tool step to stop");
        }
    });
}

fn initial_interactive_runtime(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
    agent_override: Option<&str>,
    chat_session_id: String,
) -> Result<(InteractiveChatRuntime, ikaros_harness::SkillRegistry)> {
    let agent_instance = resolve_agent_instance(config, agent_override, workspace, &paths.home)?;
    let agent = ResolvedAgentProfile {
        name: agent_instance.profile_name.clone(),
        profile: agent_instance.profile.clone(),
    };
    let (session, registry) = session_and_registry_for_instance(paths, config, &agent_instance)?;
    Ok((
        InteractiveChatRuntime {
            agent,
            agent_id: agent_instance.agent_id,
            state_dir: agent_instance.state_dir,
            workspace: agent_instance.workspace,
            session,
            chat_session_id,
            pending_inputs: VecDeque::new(),
        },
        registry,
    ))
}

fn print_chat_history(paths: &IkarosPaths, limit: usize, session_id: Option<&str>) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let store = ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let records = if let Some(session_id) = session_id {
        store.read_session(session_id)?
    } else {
        store.read_all()?
    };
    println!("chat_history: {}", store.path().display());
    println!("chat_history_backend: {}", store.backend_name());
    if let Some(session_id) = session_id {
        println!("session: {session_id}");
    }
    println!("records: {}", records.len());
    if records.is_empty() {
        println!("recent: none");
        return Ok(());
    }
    let start = records.len().saturating_sub(limit);
    print_chat_history_records("recent", &records[start..]);
    Ok(())
}

fn search_chat_history(
    paths: &IkarosPaths,
    query: &str,
    limit: usize,
    session_id: Option<&str>,
) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let store = ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let records = store.search(query, limit, session_id)?;
    println!("chat_history: {}", store.path().display());
    println!("chat_history_backend: {}", store.backend_name());
    println!("query: {}", redact_secrets(query));
    if let Some(session_id) = session_id {
        println!("session: {session_id}");
    }
    println!("records: {}", records.len());
    if records.is_empty() {
        println!("matches: none");
        return Ok(());
    }
    print_chat_history_records("matches", &records);
    Ok(())
}

fn print_chat_history_records(label: &str, records: &[ChatHistoryRecord]) {
    println!("{label}:");
    for record in records {
        println!(
            "- {} session={} turn={} agent={} provider={} model={} streamed={} context=relationship:{} memory:{} rag:{}",
            record.created_at,
            record.session_id,
            record.turn_id,
            record.agent,
            record.provider,
            record.model,
            record.streamed,
            record.relationship_hits,
            record.memory_hits,
            record.rag_hits
        );
        println!("  user: {}", record.user_message);
        println!("  assistant: {}", record.assistant_message);
    }
}

fn print_chat_sessions(paths: &IkarosPaths, limit: usize) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let store = ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let sessions = store.session_summaries(limit)?;
    println!("chat_history: {}", store.path().display());
    println!("chat_history_backend: {}", store.backend_name());
    println!("sessions: {}", sessions.len());
    if sessions.is_empty() {
        println!("recent: none");
        return Ok(());
    }
    print_chat_session_summaries(&sessions);
    Ok(())
}

fn print_chat_session_summaries(sessions: &[ChatHistorySessionSummary]) {
    println!("recent:");
    for session in sessions {
        println!(
            "- session={} turns={} first={} last={} last_turn={} agents={} providers={} models={}",
            session.session_id,
            session.turns,
            session.first_created_at,
            session.last_created_at,
            session.last_turn_id,
            session.agents.join(","),
            session.providers.join(","),
            session.models.join(",")
        );
        println!("  last_user: {}", session.last_user_message);
        println!("  last_assistant: {}", session.last_assistant_message);
        println!(
            "  continue: ikaros chat --chat-session {} --message \"...\"",
            session.session_id
        );
    }
}

fn delete_chat_history_session(paths: &IkarosPaths, session_id: &str) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let store = ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let deleted = store.delete_session(session_id)?;
    println!("chat_history: {}", store.path().display());
    println!("chat_history_backend: {}", store.backend_name());
    println!("deleted_session: {session_id}");
    println!("deleted_records: {deleted}");
    Ok(())
}

fn clear_chat_history(paths: &IkarosPaths) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let store = ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let deleted = store.clear()?;
    println!("chat_history: {}", store.path().display());
    println!("chat_history_backend: {}", store.backend_name());
    println!("deleted_records: {deleted}");
    Ok(())
}

impl From<&ChatArgs> for ChatRunOptions {
    fn from(args: &ChatArgs) -> Self {
        Self {
            stream: args.stream,
            agent_loop: !args.no_agent_loop,
            memory_limit: args.memory_limit,
            rag_top_k: args.rag_top_k,
            history_context_limit: args.history_context_limit,
            history_summary_limit: args.history_summary_limit,
            context_token_budget: args.context_token_budget,
            relationship_learning: !args.no_relationship_learning,
            scope: args.scope.clone(),
            no_context: args.no_context,
            session_id: args.chat_session.clone(),
            session_source: None,
            chat_history_path: None,
            chat_history_backend: None,
            cancellation: Default::default(),
        }
    }
}

#[cfg(test)]
mod tests;
