// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_approval_hint, print_skill_result, session_and_registry};
use anyhow::Result;
use clap::Subcommand;
use ikaros_core::IkarosPaths;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub(crate) enum FsCommand {
    Read { path: PathBuf },
    List { path: PathBuf },
    Write { path: PathBuf, content: String },
}

pub(crate) async fn fs_command(
    command: FsCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = match command {
        FsCommand::Read { path } => {
            session
                .execute_skill(&registry, "fs_read", json!({"path": path}))
                .await?
        }
        FsCommand::List { path } => {
            session
                .execute_skill(&registry, "list_dir", json!({"path": path}))
                .await?
        }
        FsCommand::Write { path, content } => {
            session
                .execute_skill(
                    &registry,
                    "fs_write_guarded",
                    json!({"path": path, "content": content}),
                )
                .await?
        }
    };
    print_skill_result(&result)?;
    if !result.ok {
        print_approval_hint(&result);
    }
    println!("audit: {}", session.audit.path().display());
    if let Some(log) = session.approvals.log() {
        println!("approvals: {}", log.path().display());
    }
    Ok(())
}
