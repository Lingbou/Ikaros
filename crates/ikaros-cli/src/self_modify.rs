// SPDX-License-Identifier: GPL-3.0-only

use crate::session_and_registry;
use anyhow::{Result, bail};
use clap::{Args, Subcommand, ValueEnum};
use ikaros_coding::{SelfModifyChangeKind, SelfModifyStore};
use ikaros_core::{IkarosConfig, IkarosPaths, RiskLevel, ToolCall, ToolResult};
use ikaros_harness::{ApprovalStatus, AuditEvent};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub(crate) enum SelfModifyCommand {
    Propose(SelfModifyPropose),
    RequestApply {
        proposal_id: String,
    },
    ApplyApproved {
        proposal_id: String,
        #[arg(long = "approval-id")]
        approval_id: String,
        #[arg(long = "check-command")]
        check_commands: Vec<String>,
    },
    Rollback {
        proposal_id: String,
    },
    List,
    Operations,
    Heartbeat,
}

#[derive(Debug, Args)]
pub(crate) struct SelfModifyPropose {
    #[arg(long, value_enum)]
    kind: SelfModifyKindArg,
    #[arg(long, value_name = "PATH")]
    target: PathBuf,
    #[arg(long)]
    diff: String,
    #[arg(long = "task-id")]
    task_id: Option<String>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum SelfModifyKindArg {
    #[value(name = "skill-patch")]
    Skill,
    #[value(name = "persona-patch")]
    Persona,
    #[value(name = "config-patch")]
    Config,
    #[value(name = "runtime-patch")]
    Runtime,
    #[value(name = "documentation-patch")]
    Documentation,
}

impl From<SelfModifyKindArg> for SelfModifyChangeKind {
    fn from(value: SelfModifyKindArg) -> Self {
        match value {
            SelfModifyKindArg::Skill => Self::SkillPatch,
            SelfModifyKindArg::Persona => Self::PersonaPatch,
            SelfModifyKindArg::Config => Self::ConfigPatch,
            SelfModifyKindArg::Runtime => Self::RuntimePatch,
            SelfModifyKindArg::Documentation => Self::DocumentationPatch,
        }
    }
}

pub(crate) async fn self_modify_command(
    command: SelfModifyCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let config = IkarosConfig::load(&paths.config)?;
    let (session, _) = session_and_registry(paths, workspace, agent_override)?;
    let store = SelfModifyStore::new(workspace, paths.home.join("self-modify"));
    match command {
        SelfModifyCommand::Propose(args) => {
            let proposal = store
                .propose_with_env(
                    args.kind.into(),
                    &args.target,
                    &args.diff,
                    args.task_id,
                    &*session.env,
                )
                .await?;
            session.audit.append(AuditEvent::new(
                "self_modify_proposal",
                None,
                "self-modify proposal recorded",
                json!({
                    "proposal_id": proposal.id,
                    "change_kind": proposal.change_kind,
                    "target_path": proposal.target_path,
                    "ok_to_request_approval": proposal.dry_run_report.ok_to_request_approval,
                    "apply_available": proposal.dry_run_report.apply_available,
                    "diff_summary": proposal.dry_run_report.diff_summary,
                }),
            )?)?;
            println!("{}", serde_json::to_string_pretty(&proposal)?);
        }
        SelfModifyCommand::RequestApply { proposal_id } => {
            let proposal = store
                .get(&proposal_id)?
                .ok_or_else(|| anyhow::anyhow!("proposal not found: {proposal_id}"))?;
            if !proposal.dry_run_report.ok_to_request_approval {
                bail!("proposal is not ready for approval-gated apply");
            }
            let call = ToolCall::new(
                "self_modify_apply",
                RiskLevel::SelfModify,
                json!({
                    "proposal_id": proposal.id,
                    "change_kind": proposal.change_kind,
                    "target_path": proposal.target_path,
                    "dry_run_report": proposal.dry_run_report,
                }),
            );
            let approval = session.approvals.enqueue(
                call,
                "self-modify apply requires explicit approval".into(),
                workspace.to_path_buf(),
                None,
            )?;
            session.audit.append(AuditEvent::new(
                "self_modify_apply_requested",
                None,
                "self-modify apply approval requested",
                json!({
                    "approval_id": approval.id,
                    "proposal_id": proposal_id,
                    "target_path": proposal.target_path,
                }),
            )?)?;
            println!("{}", serde_json::to_string_pretty(&approval)?);
            println!("approval: {}", approval.id);
            println!("next: ikaros approval approve {}", approval.id);
            println!(
                "next: ikaros self-modify apply-approved {} --approval-id {}",
                proposal_id, approval.id
            );
        }
        SelfModifyCommand::ApplyApproved {
            proposal_id,
            approval_id,
            check_commands,
        } => {
            let record = session
                .approvals
                .get(&approval_id)?
                .ok_or_else(|| anyhow::anyhow!("approval not found: {approval_id}"))?;
            if record.status != ApprovalStatus::Approved {
                bail!(
                    "approval {approval_id} is {:?}, not approved",
                    record.status
                );
            }
            if record.request.call.name != "self_modify_apply" {
                bail!("approval {approval_id} is not a self-modify apply request");
            }
            let approved_proposal_id = record
                .request
                .call
                .input
                .get("proposal_id")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| anyhow::anyhow!("approval is missing proposal_id"))?;
            if approved_proposal_id != proposal_id {
                bail!(
                    "approval {approval_id} is for proposal {approved_proposal_id}, not {proposal_id}"
                );
            }
            ensure_approval_workspace(&record.request.workspace_root, workspace, &approval_id)?;
            let report = match if check_commands.is_empty() {
                store
                    .apply_approved_with_config_and_env(
                        &proposal_id,
                        &approval_id,
                        &config.self_modify,
                        &*session.env,
                        &*session.env,
                    )
                    .await
            } else {
                store
                    .apply_approved_with_checks_and_env(
                        &proposal_id,
                        &approval_id,
                        &check_commands,
                        &*session.env,
                        &*session.env,
                    )
                    .await
            } {
                Ok(report) => report,
                Err(error) => {
                    let result = ToolResult {
                        call_id: record.request.call.id.clone(),
                        ok: false,
                        output: json!({
                            "proposal_id": proposal_id,
                            "approval_id": approval_id,
                            "error": error.to_string(),
                        }),
                        summary: "self-modify approved apply failed".into(),
                    };
                    session.approvals.mark_executed(&approval_id, result)?;
                    return Err(error.into());
                }
            };
            let result = ToolResult {
                call_id: record.request.call.id.clone(),
                ok: report.post_checks_passed,
                output: serde_json::to_value(&report)?,
                summary: if report.post_checks_passed {
                    "self-modify approved apply completed".into()
                } else {
                    "self-modify approved apply rolled back after failed self-check".into()
                },
            };
            session.approvals.mark_executed(&approval_id, result)?;
            session.audit.append(AuditEvent::new(
                "self_modify_apply",
                None,
                "self-modify approved apply completed",
                json!({
                    "approval_id": approval_id,
                    "proposal_id": proposal_id,
                    "target_path": report.target_path,
                    "check_profile": report.check_profile,
                    "patch_report": report.patch_report,
                    "pre_heartbeat": report.pre_heartbeat,
                    "post_heartbeat": report.post_heartbeat,
                    "pre_checks": report.pre_checks,
                    "post_checks": report.post_checks,
                    "post_checks_passed": report.post_checks_passed,
                    "auto_rollback": report.auto_rollback,
                }),
            )?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        SelfModifyCommand::Rollback { proposal_id } => {
            let report = store.rollback_with_env(&proposal_id, &*session.env).await?;
            session.audit.append(AuditEvent::new(
                "self_modify_rollback",
                None,
                "self-modify rollback completed",
                json!({
                    "proposal_id": proposal_id,
                    "target_path": report.target_path,
                    "snapshot_path": report.snapshot_path,
                    "restored_snapshot": report.restored_snapshot,
                    "removed_created_target": report.removed_created_target,
                }),
            )?)?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        SelfModifyCommand::List => {
            println!("{}", serde_json::to_string_pretty(&store.list()?)?);
        }
        SelfModifyCommand::Operations => {
            println!("{}", serde_json::to_string_pretty(&store.operations()?)?);
        }
        SelfModifyCommand::Heartbeat => {
            let heartbeat = store.heartbeat()?;
            session.audit.append(AuditEvent::new(
                "self_modify_heartbeat",
                None,
                "self-modify heartbeat recorded",
                json!({
                    "status": heartbeat.status,
                    "proposal_count": heartbeat.proposal_count,
                    "proposal_store": heartbeat.proposal_store,
                    "checks": heartbeat.checks,
                }),
            )?)?;
            println!("{}", serde_json::to_string_pretty(&heartbeat)?);
        }
    }
    println!("audit: {}", session.audit.path().display());
    Ok(())
}

fn ensure_approval_workspace(
    approval_workspace: &Option<PathBuf>,
    workspace: &Path,
    approval_id: &str,
) -> Result<()> {
    if let Some(expected) = approval_workspace {
        let expected = normalize_path(expected);
        let actual = normalize_path(workspace);
        if expected != actual {
            bail!(
                "approval {approval_id} was created for workspace {}, current workspace is {}",
                expected.display(),
                actual.display()
            );
        }
    }
    Ok(())
}

fn normalize_path(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}
