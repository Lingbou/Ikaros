// SPDX-License-Identifier: GPL-3.0-only

mod webhook;

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use ikaros_agent::chat::{ChatMessageContext, chat_memory_policy_from_config};
use ikaros_agent::gateway_drain::{
    GatewayChatDrainContext, GatewayMessageDrainContext, GatewayTaskDrainContext,
    GatewayWorkerTickReport, drain_gateway_message_with_context,
    record_gateway_message_preflight_failure,
};
use ikaros_agent::soul::load_or_default;
use ikaros_agent::task_loop::TaskExecutionContext;
use ikaros_core::{IkarosPaths, redact_secrets};
use ikaros_execution::harness::SkillRegistry;
use ikaros_host::{RuntimeHarness, runtime_harness, runtime_harness_model_services};
use ikaros_protocol::{
    GatewayDelivery, GatewayDeliveryStatus, GatewayMessage, GatewayMessageKind,
    GatewayMessageStatus, GatewayRoute, GatewaySessionSource,
};
use ikaros_providers::model::ModelUsageLedger;
use ikaros_state::gateway::{
    GatewayDeliveryStatusCounts, GatewayStatusSnapshot, LocalGatewayStore,
    MESSAGE_WORKER_LOCK_FILE, MESSAGE_WORKER_STOP_FILE, MessageWorkerForensics,
    acquire_message_worker_lock, clear_message_worker_stop_request, gateway_lease_is_stale,
    gateway_message_is_retryable, gateway_worker_events_path, gateway_worker_lock_path,
    gateway_worker_stop_path, latest_nonempty_line, message_daemon_log_path,
    message_daemon_status_label, redacted_json_field, redacted_message_worker_lock_owner,
    take_message_worker_stop_request, write_message_worker_stop_request,
};
use ikaros_state::memory::{JsonlMemoryJournal, LocalMemoryStore};
use ikaros_state::session::{
    RuntimeSessionTarget, SessionSource, SessionStore, SqliteSessionStore, gateway_session_id,
};
use ikaros_surfaces::gateway::builtin_gateway_adapters;
use std::{
    fs::{self, OpenOptions},
    path::Path,
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::Duration,
    time::Instant,
};
use time::OffsetDateTime;
use tokio::time::sleep;
use webhook::{MessageWebhook, serve_message_webhook};

mod adapter;
mod commands;
mod daemon;
mod delivery;
mod pairing;
mod status;
mod workbench;

use self::{adapter::*, commands::*, daemon::*, delivery::*, pairing::*, status::*};
pub(crate) use commands::{MessageCommand, message_command};
pub(crate) use ikaros_state::gateway::{
    message_worker_lock_is_stale, message_worker_lock_is_stale_label,
};
pub(crate) use status::{
    print_gateway_worker_forensics, print_gateway_worker_state, print_gateway_worker_stop,
};
pub(crate) use workbench::{
    run_gateway_adapter_workbench_command, run_gateway_daemon_workbench_command,
};

pub(crate) async fn drain_gateway_messages(
    messages: Vec<GatewayMessage>,
    store: &LocalGatewayStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Vec<ikaros_agent::gateway_drain::GatewayDrainReport>> {
    let mut reports = Vec::new();
    for message in messages {
        reports
            .push(drain_gateway_message(message, store, paths, workspace, agent_override).await?);
    }
    Ok(reports)
}

pub(crate) async fn run_gateway_worker_tick(
    store: &LocalGatewayStore,
    limit: usize,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<GatewayWorkerTickReport> {
    if limit == 0 {
        anyhow::bail!("message worker limit must be greater than zero");
    }
    let messages = store.claim_pending(limit)?;
    let pending = messages.len();
    let reports = drain_gateway_messages(messages, store, paths, workspace, agent_override).await?;
    Ok(GatewayWorkerTickReport {
        kind: "gateway_worker_tick".into(),
        pending,
        drained: reports.len(),
        reports,
    })
}

async fn drain_gateway_message(
    message: GatewayMessage,
    store: &LocalGatewayStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ikaros_agent::gateway_drain::GatewayDrainReport> {
    let agent_label = message.agent.as_deref().or(agent_override);
    let harness = match runtime_harness(paths, workspace, agent_label) {
        Ok(harness) => harness,
        Err(error) => {
            let target = fallback_gateway_session_target(paths, workspace, agent_label);
            return Ok(record_gateway_message_preflight_failure(
                message, store, &target, error,
            )?);
        }
    };
    let session_target = RuntimeSessionTarget {
        store: SqliteSessionStore::new(harness.agent_instance.state_dir.clone()),
        agent_id: harness.agent_instance.agent_id.clone(),
        workspace: harness.agent_instance.workspace.clone(),
    };
    match message.kind {
        GatewayMessageKind::Chat => {
            let persona = match load_or_default(&paths.persona_dir) {
                Ok(persona) => persona,
                Err(error) => {
                    return Ok(record_gateway_message_preflight_failure(
                        message,
                        store,
                        &session_target,
                        error,
                    )?);
                }
            };
            let model = match runtime_harness_model_services(paths, &harness) {
                Ok(model) => model,
                Err(error) => {
                    return Ok(record_gateway_message_preflight_failure(
                        message,
                        store,
                        &session_target,
                        error,
                    )?);
                }
            };
            let session_store: Arc<dyn SessionStore> =
                Arc::new(SqliteSessionStore::new(&harness.agent_instance.state_dir));
            let memory_provider =
                match LocalMemoryStore::new(&paths.memory_dir, &harness.config.memory.backend) {
                    Ok(store) => store,
                    Err(error) => {
                        return Ok(record_gateway_message_preflight_failure(
                            message,
                            store,
                            &session_target,
                            error,
                        )?);
                    }
                };
            let memory_journal = JsonlMemoryJournal::new(&paths.memory_dir);
            let memory_policy = chat_memory_policy_from_config(&harness.config.memory.policy);
            let usage_ledger = ModelUsageLedger::new(&paths.audit_dir);
            let session_state_db = harness.agent_instance.state_dir.join("state.db");
            let RuntimeHarness {
                agent,
                agent_instance,
                session,
                registry,
                ..
            } = harness;
            drain_gateway_message_with_context(
                message,
                store,
                GatewayMessageDrainContext::Chat(GatewayChatDrainContext {
                    session_target: &session_target,
                    chat: ChatMessageContext {
                        persona: &persona,
                        provider: model.provider.as_ref(),
                        agent: &agent,
                        session: &session,
                        registry: &registry,
                        session_store,
                        session_source: SessionSource::Service {
                            name: "gateway".into(),
                        },
                        agent_id: &agent_instance.agent_id,
                        workspace: &agent_instance.workspace,
                        memory_provider: &memory_provider,
                        memory_journal: &memory_journal,
                        memory_policy: &memory_policy,
                        request_options: &model.request_options,
                        model_usage_path: usage_ledger.path().to_path_buf(),
                        session_state_db,
                    },
                }),
            )
            .await
            .map_err(Into::into)
        }
        GatewayMessageKind::Task => {
            let RuntimeHarness {
                agent,
                session,
                registry,
                ..
            } = harness;
            drain_task_gateway_message(message, store, session_target, agent, session, registry)
                .await
        }
    }
}

async fn drain_task_gateway_message(
    message: GatewayMessage,
    store: &LocalGatewayStore,
    session_target: RuntimeSessionTarget,
    agent: ikaros_core::ResolvedAgentProfile,
    session: ikaros_execution::harness::ExecutionSession,
    registry: SkillRegistry,
) -> Result<ikaros_agent::gateway_drain::GatewayDrainReport> {
    drain_gateway_message_with_context(
        message,
        store,
        GatewayMessageDrainContext::Task(GatewayTaskDrainContext {
            session_target: &session_target,
            task: TaskExecutionContext {
                agent: &agent,
                session,
                registry,
                agent_loop: None,
            },
        }),
    )
    .await
    .map_err(Into::into)
}

fn fallback_gateway_session_target(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_label: Option<&str>,
) -> RuntimeSessionTarget {
    let agent_id = sanitize_runtime_path_segment(agent_label.unwrap_or("build"));
    RuntimeSessionTarget {
        store: SqliteSessionStore::new(paths.home.join("agents").join(&agent_id)),
        agent_id,
        workspace: workspace.to_path_buf(),
    }
}

fn sanitize_runtime_path_segment(value: &str) -> String {
    let sanitized = redact_secrets(value).replace(['/', '\\', ':', '\n', '\r', '\t'], "_");
    if sanitized.trim().is_empty() {
        "build".into()
    } else {
        sanitized
    }
}
