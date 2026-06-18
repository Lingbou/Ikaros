// SPDX-License-Identifier: GPL-3.0-only

mod interactive;
mod output;

use crate::{resolve_agent_instance, session_and_registry_for_instance};
use anyhow::Result;
use clap::Args;
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile, redact_secrets};
use ikaros_models::{ModelUsageLedger, governed_provider_from_config};
use ikaros_runtime::{
    ChatHistoryRecord, ChatHistorySessionSummary, ChatHistoryStore, ChatRunOptions,
    DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET, new_chat_session_id, run_chat_message, run_chat_turn,
};
use ikaros_soul::load_or_default;
use std::{
    io::{self, Write},
    path::Path,
};

use interactive::{InteractiveChatRuntime, handle_interactive_chat_command};
use output::{print_chat_content, print_chat_message_result};

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
    let provider = governed_provider_from_config(
        &config.model.default,
        &config.providers.model,
        &paths.audit_dir,
    )?;
    let history_store =
        ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let mut options = ChatRunOptions::from(&args);
    let chat_session_id = options
        .session_id
        .clone()
        .unwrap_or_else(new_chat_session_id);
    options.session_id = Some(chat_session_id.clone());
    options.chat_history_path = Some(history_store.path().to_path_buf());
    options.chat_history_backend = Some(history_store.backend_name().into());
    let (mut runtime, registry) =
        initial_interactive_runtime(paths, workspace, &config, agent_override, chat_session_id)?;
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
        if input.starts_with('/') {
            handle_interactive_chat_command(
                input,
                &config,
                paths,
                workspace,
                &mut runtime,
                &options,
                &usage_ledger,
            )
            .await?;
            continue;
        }
        if input.is_empty() {
            continue;
        }
        let report = run_chat_turn(
            input,
            &persona,
            provider.as_ref(),
            &runtime.agent,
            &runtime.session,
            &registry,
            &options,
        )
        .await?;
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
        print_chat_content(&report)?;
    }
    Ok(())
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
            session,
            chat_session_id,
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
        }
    }
}

#[cfg(test)]
mod tests;
