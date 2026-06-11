// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_skill_result, session_and_registry};
use anyhow::Result;
use clap::Subcommand;
use ikaros_core::IkarosPaths;
use serde_json::json;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum RepoCommand {
    Scan,
}

pub(crate) async fn repo_command(
    command: RepoCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = match command {
        RepoCommand::Scan => {
            session
                .execute_skill(&registry, "repo_scan", json!({}))
                .await?
        }
    };
    print_skill_result(&result)?;
    println!("audit: {}", session.audit.path().display());
    Ok(())
}
