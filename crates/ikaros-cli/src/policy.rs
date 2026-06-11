// SPDX-License-Identifier: GPL-3.0-only

use crate::session_and_registry;
use anyhow::Result;
use clap::{Args, Subcommand, ValueEnum};
use ikaros_core::{IkarosPaths, RiskLevel};
use ikaros_harness::PolicyRequest;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub(crate) enum PolicyCommand {
    Explain(PolicyExplain),
}

#[derive(Debug, Args)]
pub(crate) struct PolicyExplain {
    action: String,
    #[arg(long, value_enum, default_value = "safe-read")]
    risk: RiskArg,
    #[arg(long)]
    path: Option<PathBuf>,
    #[arg(long)]
    command: Option<String>,
    #[arg(long)]
    write: bool,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum RiskArg {
    SafeRead,
    LocalWrite,
    ShellRead,
    ShellWrite,
    Network,
    DatabaseWrite,
    RemoteServer,
    Destructive,
    SecretAccess,
    SelfModify,
}

impl From<RiskArg> for RiskLevel {
    fn from(value: RiskArg) -> Self {
        match value {
            RiskArg::SafeRead => Self::SafeRead,
            RiskArg::LocalWrite => Self::LocalWrite,
            RiskArg::ShellRead => Self::ShellRead,
            RiskArg::ShellWrite => Self::ShellWrite,
            RiskArg::Network => Self::Network,
            RiskArg::DatabaseWrite => Self::DatabaseWrite,
            RiskArg::RemoteServer => Self::RemoteServer,
            RiskArg::Destructive => Self::Destructive,
            RiskArg::SecretAccess => Self::SecretAccess,
            RiskArg::SelfModify => Self::SelfModify,
        }
    }
}

pub(crate) fn policy_command(
    command: PolicyCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        PolicyCommand::Explain(args) => explain_policy(args, paths, workspace, agent_override)?,
    }
    Ok(())
}

fn explain_policy(
    args: PolicyExplain,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (session, _) = session_and_registry(paths, workspace, agent_override)?;
    let session = session.with_explain(true);
    let request = PolicyRequest {
        action: args.action,
        risk: args.risk.into(),
        path: args.path,
        command: args.command,
        is_write: args.write,
    };
    let evaluation = session.evaluate(&request)?;
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "request": request,
            "evaluation": evaluation,
            "sandbox": {
                "workspace_root": session.sandbox.workspace_root,
                "dry_run": session.sandbox.dry_run,
                "explain": session.sandbox.explain,
                "agent": session.sandbox.agent,
                "protected_paths": session.sandbox.protected_paths,
            },
            "audit": session.audit.path(),
        }))?
    );
    Ok(())
}
