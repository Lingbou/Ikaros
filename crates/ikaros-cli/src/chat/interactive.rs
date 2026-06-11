// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, anyhow};
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile, redact_secrets};
use ikaros_harness::{AuditEvent, ExecutionSession};
use ikaros_models::ModelUsageLedger;
use ikaros_runtime::{ChatHistoryStore, ChatRunOptions, base_body_status};
use serde_json::json;
use std::path::Path;

pub(super) struct InteractiveChatRuntime {
    pub(super) agent: ResolvedAgentProfile,
    pub(super) session: ExecutionSession,
    pub(super) chat_session_id: String,
}

pub(super) fn handle_interactive_chat_command(
    input: &str,
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &mut InteractiveChatRuntime,
    options: &ChatRunOptions,
    usage_ledger: &ModelUsageLedger,
) -> Result<()> {
    let mut parts = input.split_whitespace();
    let command = parts.next().unwrap_or_default();
    match command {
        "/help" => {
            println!("commands: /agents, /agent <profile>, /status, /quit");
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
            let new_agent = resolve_interactive_agent(config, requested)?;
            runtime.session =
                ExecutionSession::new_with_agent(workspace, &paths.audit_dir, &new_agent);
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
            println!(
                "agent: {} mode={} workspace_writes={} shell={} network={}",
                redact_secrets(&runtime.agent.name),
                runtime.agent.mode(),
                runtime.agent.profile.workspace_writes,
                runtime.agent.profile.shell,
                runtime.agent.profile.network
            );
        }
        "/status" => {
            let history_store =
                ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
            let body_status = base_body_status(paths)?;
            println!(
                "{}",
                format_interactive_chat_status(
                    &runtime.agent,
                    &runtime.session,
                    &runtime.chat_session_id,
                    options,
                    &body_status.emotion,
                    usage_ledger,
                    &history_store,
                )
            );
        }
        _ => {
            println!("unknown command: {command}. Type /help for commands.");
        }
    }
    Ok(())
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
                redact_secrets(name),
                profile.mode,
                profile.workspace_writes,
                profile.shell,
                profile.network,
                redact_secrets(&profile.description)
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
        "agent={} mode={} emotion={} memory_context={} rag_context={} history_context_limit={} history_summary_limit={} context_char_budget={} relationship_learning={} agent_loop={} stream={} no_context={} scope={} chat_session={} audit={} model_usage={} chat_history={}",
        redact_secrets(&agent.name),
        agent.mode(),
        redact_secrets(emotion),
        agent.profile.memory_context,
        agent.profile.rag_context,
        options.history_context_limit,
        options.history_summary_limit,
        options.context_char_budget,
        options.relationship_learning,
        options.agent_loop,
        options.stream,
        options.no_context,
        options
            .scope
            .as_deref()
            .map(redact_secrets)
            .unwrap_or_else(|| "none".into()),
        redact_secrets(chat_session_id),
        session.audit.path().display(),
        usage_ledger.path().display(),
        history_store.path().display(),
    )
}
