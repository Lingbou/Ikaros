// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_skill_result, session_and_registry};
use anyhow::Result;
use clap::Subcommand;
use ikaros_core::IkarosPaths;
use serde_json::json;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum TestCommand {
    Infer,
    Run {
        #[arg(long)]
        command: Option<String>,
    },
}

pub(crate) async fn test_command(
    command: TestCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = match command {
        TestCommand::Infer => {
            session
                .execute_skill(&registry, "run_tests", json!({"infer": true}))
                .await?
        }
        TestCommand::Run { command } => {
            let mut input = json!({});
            if let Some(command) = command {
                input["command"] = json!(command);
            }
            session.execute_skill(&registry, "run_tests", input).await?
        }
    };
    print_skill_result(&result)?;
    println!("audit: {}", session.audit.path().display());
    Ok(())
}
