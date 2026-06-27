// SPDX-License-Identifier: GPL-3.0-only

use crate::resolve_agent_instance;
use anyhow::{Result, anyhow};
use clap::{Args, Subcommand, ValueEnum};
use ikaros_core::{
    AgentInstance, IkarosConfig, IkarosPaths, ModelConfig, STRUCTURED_TRACE_SCHEMA, redact_json,
    redact_secrets,
};
use ikaros_gateway::{
    GatewayDeliveryStatus, GatewayMessageStatus, GatewayPairingStatus, LocalGatewayStore,
};
use ikaros_harness::{
    AuditLog, ProcessRequest, SandboxDebugReport, local_sandbox_debug_report,
    sandbox_isolation_matrix,
};
use ikaros_memory::{JsonlMemoryJournal, MemoryJournal, MemoryJournalEntry, MemoryRef};
use ikaros_models::{
    ModelProviderDescriptor, ModelUsageLedger, ModelUsageRecord, ProviderHealthLedger,
    ProviderRegistry,
};
use ikaros_runtime::{provider_egress_allowed_hosts, runtime_doctor_report, runtime_execution_env};
use ikaros_session::{
    AgentEvent, AgentEventKind, IKAROS_PROTOCOL_NAME, IKAROS_PROTOCOL_VERSION, SessionContinuation,
    SessionContinuationStatus, SessionContinuationStatusReason, SessionId, SessionReplay,
    SessionReplayPage, SessionStore, SqliteSessionStore, StateTraceEntry, TurnStateSnapshot,
    agent_events_to_state_trace,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

mod coding;
mod continuations;
mod dump;
mod insights;
mod logs;
mod memory;
mod provider;
mod readiness;
mod sandbox;
mod session;
mod state_db;
mod trace;

use self::{
    coding::*, continuations::*, dump::*, insights::*, logs::*, memory::*, provider::*,
    readiness::*, sandbox::*, session::*, state_db::*, trace::*,
};

pub(crate) use self::{
    dump::debug_dump_json_line, insights::debug_insights_json_line, logs::debug_logs_json_line,
    memory::debug_memory_lifecycle_json_line, readiness::debug_readiness_json_line,
    sandbox::debug_sandbox_json_line, state_db::debug_state_db_json_line,
};

#[derive(Debug, Subcommand)]
pub(crate) enum DebugCommand {
    ContextDiff(DebugSessionQuery),
    MemoryLifecycle(DebugSessionQuery),
    Continuations(DebugSessionQuery),
    CodingTurn(DebugSessionQuery),
    Trace(DebugSessionQuery),
    Session(DebugSessionArgs),
    StateDb(DebugStateDbArgs),
    Logs(DebugLogsArgs),
    Insights,
    Readiness,
    Dump(DebugDumpArgs),
    Sandbox(DebugSandboxArgs),
    Provider,
}

#[derive(Debug, Args)]
pub(crate) struct DebugSessionQuery {
    session_id: String,
    #[arg(long)]
    turn_id: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct DebugSessionArgs {
    session_id: String,
    #[arg(long, default_value_t = 1)]
    page: usize,
    #[arg(long, default_value_t = 50)]
    page_size: usize,
    #[arg(long, value_name = "PATH")]
    export: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub(crate) struct DebugStateDbArgs {
    #[arg(long)]
    checkpoint: bool,
    #[arg(long, value_name = "PATH")]
    backup: Option<PathBuf>,
    #[arg(long, value_name = "PATH")]
    repair: Option<PathBuf>,
    #[arg(long, value_name = "PATH")]
    restore: Option<PathBuf>,
    #[arg(long, value_name = "RFC3339")]
    prune_ended_before: Option<String>,
    #[arg(long)]
    vacuum: bool,
}

#[derive(Debug, Args)]
pub(crate) struct DebugLogsArgs {
    #[arg(long, value_enum, default_value_t = DebugLogSource::All)]
    source: DebugLogSource,
    #[arg(long, default_value_t = 1)]
    page: usize,
    #[arg(long, default_value_t = 50)]
    page_size: usize,
}

#[derive(Debug, Args)]
pub(crate) struct DebugDumpArgs {
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,
    #[arg(long, default_value_t = 50)]
    recent_logs: usize,
}

#[derive(Debug, Args)]
pub(crate) struct DebugSandboxArgs {
    #[arg(long)]
    pub(crate) probe: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub(crate) enum DebugLogSource {
    All,
    Audit,
    ModelUsage,
    Trace,
}

impl DebugLogSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Audit => "audit",
            Self::ModelUsage => "model_usage",
            Self::Trace => "trace",
        }
    }

    fn includes_audit(self) -> bool {
        matches!(self, Self::All | Self::Audit)
    }

    fn includes_model_usage(self) -> bool {
        matches!(self, Self::All | Self::ModelUsage)
    }

    fn includes_trace(self) -> bool {
        matches!(self, Self::All | Self::Trace)
    }
}

pub(crate) async fn debug_command(
    command: DebugCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        DebugCommand::ContextDiff(args) => {
            debug_context_diff(args, paths, workspace, agent_override)
        }
        DebugCommand::MemoryLifecycle(args) => {
            debug_memory_lifecycle(args, paths, workspace, agent_override)
        }
        DebugCommand::Continuations(args) => {
            debug_continuations(args, paths, workspace, agent_override)
        }
        DebugCommand::CodingTurn(args) => debug_coding_turn(args, paths, workspace, agent_override),
        DebugCommand::Trace(args) => debug_trace(args, paths, workspace, agent_override),
        DebugCommand::Session(args) => debug_session(args, paths, workspace, agent_override),
        DebugCommand::StateDb(args) => debug_state_db(args, paths, workspace, agent_override),
        DebugCommand::Logs(args) => debug_logs(args, paths),
        DebugCommand::Insights => debug_insights(paths, workspace, agent_override),
        DebugCommand::Readiness => debug_readiness(paths, workspace, agent_override),
        DebugCommand::Dump(args) => debug_dump(args, paths, workspace, agent_override),
        DebugCommand::Sandbox(args) => debug_sandbox(args, paths, workspace, agent_override).await,
        DebugCommand::Provider => debug_provider(paths, workspace, agent_override),
    }
}
