// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_skill_result, session_and_registry};
use anyhow::Result;
use clap::Subcommand;
use ikaros_core::IkarosPaths;
use ikaros_harness::ApprovalStatus;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum ApprovalCommand {
    List {
        #[arg(long)]
        all: bool,
    },
    Approve {
        id: String,
        #[arg(long)]
        note: Option<String>,
    },
    Deny {
        id: String,
        #[arg(long)]
        note: Option<String>,
    },
}

pub(crate) async fn approval_command(
    command: ApprovalCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    match command {
        ApprovalCommand::List { all } => {
            let records = if all {
                session.approval_records()?
            } else {
                session.pending_approvals()?
            };
            println!("{}", serde_json::to_string_pretty(&records)?);
        }
        ApprovalCommand::Approve { id, note } => {
            let record = session.decide_approval(&id, ApprovalStatus::Approved, note)?;
            println!("{}", serde_json::to_string_pretty(&record)?);
            if record.request.call.name == "self_modify_apply" {
                let proposal_id = record
                    .request
                    .call
                    .input
                    .get("proposal_id")
                    .and_then(serde_json::Value::as_str);
                if let Some(proposal_id) = proposal_id {
                    println!(
                        "next: ikaros self-modify apply-approved {} --approval-id {}",
                        proposal_id, id
                    );
                } else {
                    println!(
                        "next: ikaros self-modify apply-approved <proposal-id> --approval-id {id}"
                    );
                }
                println!("approval is approved but not executed");
                println!("audit: {}", session.audit.path().display());
                if let Some(log) = session.approvals.log() {
                    println!("approvals: {}", log.path().display());
                }
                return Ok(());
            }
            let result = session.execute_approved_skill(&registry, &id).await?;
            print_skill_result(&result)?;
        }
        ApprovalCommand::Deny { id, note } => {
            let record = session.decide_approval(&id, ApprovalStatus::Denied, note)?;
            println!("{}", serde_json::to_string_pretty(&record)?);
        }
    }
    println!("audit: {}", session.audit.path().display());
    if let Some(log) = session.approvals.log() {
        println!("approvals: {}", log.path().display());
    }
    Ok(())
}
