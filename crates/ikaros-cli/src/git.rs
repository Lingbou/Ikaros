// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_skill_result, session_and_registry};
use anyhow::Result;
use clap::Subcommand;
use ikaros_core::IkarosPaths;
use serde_json::json;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum GitCommand {
    Status,
    Diff {
        #[arg(long)]
        stat: bool,
    },
}

pub(crate) async fn git_command(
    command: GitCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = match command {
        GitCommand::Status => {
            session
                .execute_skill(&registry, "git_status", json!({}))
                .await?
        }
        GitCommand::Diff { stat } => {
            session
                .execute_skill(&registry, "git_diff", json!({"stat": stat}))
                .await?
        }
    };
    print_skill_result(&result)?;
    println!("audit: {}", session.audit.path().display());
    Ok(())
}
