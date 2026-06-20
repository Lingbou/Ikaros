// SPDX-License-Identifier: GPL-3.0-only

use crate::code::{code_command, parse_interactive_code_command};
use crate::provider::{ProviderCommand, provider_command};
use crate::resolve_agent_instance;
use anyhow::{Context, Result, anyhow};
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile};
use ikaros_harness::{AuditEvent, ExecutionSession};
use ikaros_models::ModelUsageLedger;
use ikaros_runtime::{
    ChatHistoryStore, ChatRunOptions, new_chat_session_id, runtime_execution_env,
};
use ikaros_session::{
    SessionBranchSummaryInput, SessionEntry, SessionEntryKind, SessionId, SessionStore,
    SqliteSessionStore,
};
use serde_json::json;
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
};

use super::workbench::{
    TimelineVerbosity, format_workbench_help, normalize_session_id, print_approval_status,
    print_context_mentions, print_context_status, print_diff_status, print_gateway_status,
    print_memory_status, print_rag_status, print_replay_status, print_session_history,
    print_session_status, print_session_summaries, print_slash_commands, print_tasks_status,
    print_trace_status, print_workbench_status, suggest_slash_command, terminal_inline,
};

pub(super) struct InteractiveChatRuntime {
    pub(super) agent: ResolvedAgentProfile,
    pub(super) agent_id: String,
    pub(super) state_dir: PathBuf,
    pub(super) workspace: PathBuf,
    pub(super) session: ExecutionSession,
    pub(super) chat_session_id: String,
    pub(super) pending_inputs: VecDeque<String>,
}

pub(super) async fn handle_interactive_chat_command(
    input: &str,
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &mut InteractiveChatRuntime,
    options: &mut ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    let mut parts = input.split_whitespace();
    let command = parts.next().unwrap_or_default();
    match command {
        "/help" => {
            println!("{}", format_workbench_help());
        }
        "/commands" => {
            print_slash_commands(parts.next());
        }
        "/queue" => {
            handle_queue_command(parts.collect::<Vec<_>>(), runtime);
        }
        "/agents" => {
            for line in available_agent_lines(config, &runtime.agent.name) {
                println!("{line}");
            }
        }
        "/agent" => {
            let requested = parts
                .next()
                .ok_or_else(|| anyhow!("usage: /agent <profile>"))?;
            let agent_instance =
                resolve_agent_instance(config, Some(requested), workspace, &paths.home)?;
            let new_agent = resolve_interactive_agent(config, requested)?;
            runtime.session = ExecutionSession::new_with_agent_instance(
                workspace,
                &paths.audit_dir,
                &agent_instance,
            )
            .with_execution_env(runtime_execution_env(config, workspace)?);
            runtime.session.audit.append(AuditEvent::new(
                "chat_agent_switch",
                None,
                format!("chat agent switched to {}", new_agent.name),
                json!({
                    "agent": &new_agent.name,
                    "agent_mode": new_agent.mode().as_str(),
                    "workspace_writes": new_agent.profile.workspace_writes.as_str(),
                    "shell": new_agent.profile.shell.as_str(),
                    "network": new_agent.profile.network.as_str(),
                }),
            )?)?;
            runtime.agent = new_agent;
            runtime.agent_id = agent_instance.agent_id;
            runtime.state_dir = agent_instance.state_dir;
            runtime.workspace = agent_instance.workspace;
            println!(
                "agent: {} mode={} workspace_writes={} shell={} network={}",
                terminal_inline(&runtime.agent.name),
                runtime.agent.mode(),
                runtime.agent.profile.workspace_writes,
                runtime.agent.profile.shell,
                runtime.agent.profile.network
            );
        }
        "/status" => {
            print_workbench_status(config, paths, workspace, runtime, options, usage_ledger)?;
        }
        "/sessions" => {
            print_session_summaries(config, paths, 20)?;
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
        "/new" => {
            let session_id = new_chat_session_id();
            runtime.chat_session_id = session_id.clone();
            options.session_id = Some(session_id.clone());
            println!("session_new: {}", terminal_inline(&session_id));
        }
        "/fork" => {
            handle_fork_command(parts.collect::<Vec<_>>(), runtime)?;
        }
        "/timeline" => {
            print_replay_status(
                "timeline",
                config,
                paths,
                workspace,
                runtime,
                TimelineVerbosity::Timeline,
            )?;
        }
        "/replay" => {
            print_replay_status(
                "replay",
                config,
                paths,
                workspace,
                runtime,
                TimelineVerbosity::Replay,
            )?;
        }
        "/debug" => {
            print_replay_status(
                "debug",
                config,
                paths,
                workspace,
                runtime,
                TimelineVerbosity::Debug,
            )?;
        }
        "/trace" => {
            print_trace_status(config, paths, workspace, runtime)?;
        }
        "/mentions" => {
            print_context_mentions(workspace, parts.next())?;
        }
        "/provider" => {
            let args = parts.collect::<Vec<_>>();
            handle_provider_command(args.clone(), paths, workspace).await?;
            append_workbench_evidence(runtime, "provider", json!({"args": args}))?;
        }
        "/model" => {
            handle_provider_command(vec!["inspect"], paths, workspace).await?;
            append_workbench_evidence(runtime, "model", json!({"args": ["inspect"]}))?;
        }
        "/gateway" => {
            print_gateway_status(paths)?;
            append_workbench_evidence(runtime, "gateway", json!({}))?;
        }
        "/tasks" => {
            print_tasks_status(paths)?;
            append_workbench_evidence(runtime, "tasks", json!({}))?;
        }
        "/approval" | "/approvals" => {
            print_approval_status(runtime)?;
        }
        "/context" => {
            print_context_status(runtime, options)?;
        }
        "/memory" => {
            print_memory_status(config, paths, runtime)?;
        }
        "/rag" => {
            print_rag_status(config, paths, options);
        }
        "/diff" => {
            print_diff_status(runtime, workspace).await?;
        }
        "/clear" => {
            println!("screen_cleared: true");
        }
        "/code" => {
            let command_line = input
                .strip_prefix("/code")
                .map(str::trim)
                .unwrap_or_default();
            if command_line.is_empty() {
                println!("usage: /code <plan|apply|test|review|rollback> ...");
            } else {
                let command = parse_interactive_code_command(command_line)
                    .with_context(|| "failed to parse /code command")?;
                code_command(command, paths, workspace, Some(&runtime.agent.name)).await?;
            }
        }
        _ => {
            println!(
                "unknown command: {}. Type /help for commands.",
                terminal_inline(command)
            );
            if let Some(suggestion) = suggest_slash_command(command) {
                println!("did_you_mean: {suggestion}");
            }
        }
    }
    Ok(())
}

fn append_workbench_evidence(
    runtime: &InteractiveChatRuntime,
    kind: &str,
    payload: serde_json::Value,
) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let parent_entry_id = store
        .get_session(&session_id)?
        .and_then(|session| session.active_leaf_entry_id);
    let mut entry = SessionEntry::new(session_id.clone(), SessionEntryKind::Custom);
    entry.parent_entry_id = parent_entry_id;
    entry.visible_text = Some(format!("workbench {kind} status queried"));
    entry.payload = json!({
        "operation": "workbench_evidence",
        "kind": kind,
        "session_id": session_id.as_str(),
        "agent_id": &runtime.agent_id,
        "workspace": runtime.workspace.display().to_string(),
        "data": payload,
    });
    store.append_entry(&entry)?;
    println!(
        "workbench_evidence: kind={} entry={}",
        terminal_inline(kind),
        terminal_inline(entry.entry_id.as_str())
    );
    Ok(())
}

fn handle_queue_command(args: Vec<&str>, runtime: &mut InteractiveChatRuntime) {
    match args.as_slice() {
        [] => {
            println!("pending_inputs: {}", runtime.pending_inputs.len());
            for (index, input) in runtime.pending_inputs.iter().enumerate() {
                println!("- index={} message={}", index + 1, terminal_inline(input));
            }
        }
        ["clear"] => {
            let cleared = runtime.pending_inputs.len();
            runtime.pending_inputs.clear();
            println!("pending_inputs_cleared: {cleared}");
        }
        _ => {
            let input = args.join(" ");
            runtime.pending_inputs.push_back(input);
            println!("pending_input_queued: {}", runtime.pending_inputs.len());
        }
    }
}

fn handle_fork_command(args: Vec<&str>, runtime: &mut InteractiveChatRuntime) -> Result<()> {
    let summary = if args.is_empty() {
        "workbench fork from active leaf".to_owned()
    } else {
        args.join(" ")
    };
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let Some(session) = store.get_session(&session_id)? else {
        println!("session_fork: not_found");
        println!("session: {}", terminal_inline(session_id.as_str()));
        println!("reason: no persisted session timeline found");
        return Ok(());
    };
    let Some(parent_entry_id) = session.active_leaf_entry_id else {
        println!("session_fork: unavailable");
        println!("session: {}", terminal_inline(session_id.as_str()));
        println!("reason: session has no active leaf");
        return Ok(());
    };
    let entry = store.branch_from_entry(&SessionBranchSummaryInput {
        session_id: session_id.clone(),
        parent_entry_id: parent_entry_id.clone(),
        summary: summary.clone(),
        payload: json!({
            "source": "workbench",
            "command": "/fork",
            "agent_id": &runtime.agent_id,
            "workspace": runtime.workspace.display().to_string(),
        }),
    })?;
    println!("session_forked: {}", terminal_inline(session_id.as_str()));
    println!(
        "fork_parent_entry: {}",
        terminal_inline(parent_entry_id.as_str())
    );
    println!("fork_entry: {}", terminal_inline(entry.entry_id.as_str()));
    println!("fork_summary: {}", terminal_inline(&summary));
    Ok(())
}

fn handle_session_command(
    args: Vec<&str>,
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &mut InteractiveChatRuntime,
    options: &mut ChatRunOptions,
) -> Result<()> {
    match args.as_slice() {
        [] | ["status"] => {
            print_session_status(config, paths, runtime, options)?;
        }
        ["resume", session_id] => {
            let session_id = normalize_session_id(session_id);
            if session_id.is_empty() {
                return Err(anyhow!("usage: /session resume <session-id>"));
            }
            runtime.chat_session_id = session_id.clone();
            options.session_id = Some(session_id.clone());
            println!("session_resumed: {}", terminal_inline(&session_id));
        }
        ["history"] => {
            print_session_history(config, paths, &runtime.chat_session_id, 10)?;
        }
        ["history", limit] => {
            let limit = limit
                .parse::<usize>()
                .with_context(|| "session history limit must be a positive number")?;
            print_session_history(config, paths, &runtime.chat_session_id, limit)?;
        }
        ["timeline"] => {
            print_replay_status(
                "timeline",
                config,
                paths,
                workspace,
                runtime,
                TimelineVerbosity::Timeline,
            )?;
        }
        _ => {
            println!("usage: /session status|resume <session-id>|history [limit]|timeline");
        }
    }
    Ok(())
}

async fn handle_provider_command(
    args: Vec<&str>,
    paths: &IkarosPaths,
    workspace: &Path,
) -> Result<()> {
    match args.as_slice() {
        [] | ["inspect"] => provider_command(ProviderCommand::Inspect, paths, workspace).await,
        ["health"] => {
            provider_command(ProviderCommand::Health { live: false }, paths, workspace).await
        }
        ["health", "--live"] => {
            provider_command(ProviderCommand::Health { live: true }, paths, workspace).await
        }
        ["matrix"] => {
            provider_command(ProviderCommand::Matrix { live: false }, paths, workspace).await
        }
        ["matrix", "--live"] => {
            provider_command(ProviderCommand::Matrix { live: true }, paths, workspace).await
        }
        _ => {
            println!("usage: /provider [inspect|health [--live]|matrix [--live]]");
            Ok(())
        }
    }
}

pub(super) fn resolve_interactive_agent(
    config: &IkarosConfig,
    requested: &str,
) -> Result<ResolvedAgentProfile> {
    config
        .agent
        .resolve(Some(requested))
        .ok_or_else(|| anyhow!("agent profile not found: {requested}"))
}

pub(super) fn available_agent_lines(config: &IkarosConfig, active: &str) -> Vec<String> {
    config
        .agent
        .profiles
        .iter()
        .map(|(name, profile)| {
            let marker = if name == active { "*" } else { " " };
            format!(
                "{marker} {} mode={} workspace_writes={} shell={} network={} - {}",
                terminal_inline(name),
                profile.mode,
                profile.workspace_writes,
                profile.shell,
                profile.network,
                terminal_inline(&profile.description)
            )
        })
        .collect()
}

pub(super) fn format_interactive_chat_status(
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    chat_session_id: &str,
    options: &ChatRunOptions,
    emotion: &str,
    usage_ledger: &ModelUsageLedger,
    history_store: &ChatHistoryStore,
) -> String {
    format!(
        "agent={} mode={} emotion={} memory_context={} rag_context={} history_context_limit={} history_summary_limit={} context_token_budget={} relationship_learning={} agent_loop={} stream={} no_context={} scope={} chat_session={} audit={} model_usage={} chat_history={}",
        terminal_inline(&agent.name),
        agent.mode(),
        terminal_inline(emotion),
        agent.profile.memory_context,
        agent.profile.rag_context,
        options.history_context_limit,
        options.history_summary_limit,
        options.context_token_budget,
        options.relationship_learning,
        options.agent_loop,
        options.stream,
        options.no_context,
        options
            .scope
            .as_deref()
            .map(terminal_inline)
            .unwrap_or_else(|| "none".into()),
        terminal_inline(chat_session_id),
        terminal_inline(&session.audit.path().display().to_string()),
        terminal_inline(&usage_ledger.path().display().to_string()),
        terminal_inline(&history_store.path().display().to_string()),
    )
}
