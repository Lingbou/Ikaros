// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_skill_result, session_and_registry};
use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_core::IkarosPaths;
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub(crate) enum RagCommand {
    Ingest(RagIngest),
    Search(RagSearch),
    Stale,
    DeleteScope { scope: String },
    DeletePath { path: PathBuf },
    Reindex(RagIngest),
}

#[derive(Debug, Args)]
pub(crate) struct RagIngest {
    path: PathBuf,
    #[arg(long, default_value = "project")]
    scope: String,
}

#[derive(Debug, Args)]
pub(crate) struct RagSearch {
    query: String,
    #[arg(long)]
    scope: Option<String>,
    #[arg(long, default_value_t = 5)]
    top_k: usize,
}

pub(crate) async fn rag_command(
    command: RagCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = match command {
        RagCommand::Ingest(args) => {
            session
                .execute_skill(
                    &registry,
                    "rag_ingest",
                    json!({"path": args.path, "scope": args.scope}),
                )
                .await?
        }
        RagCommand::Search(args) => {
            let mut input = json!({"query": args.query, "top_k": args.top_k});
            if let Some(scope) = args.scope {
                input["scope"] = json!(scope);
            }
            session
                .execute_skill(&registry, "rag_search", input)
                .await?
        }
        RagCommand::Stale => {
            session
                .execute_skill(&registry, "rag_stale", json!({}))
                .await?
        }
        RagCommand::DeleteScope { scope } => {
            session
                .execute_skill(&registry, "rag_delete_scope", json!({"scope": scope}))
                .await?
        }
        RagCommand::DeletePath { path } => {
            session
                .execute_skill(&registry, "rag_delete_path", json!({"path": path}))
                .await?
        }
        RagCommand::Reindex(args) => {
            session
                .execute_skill(
                    &registry,
                    "rag_reindex",
                    json!({"path": args.path, "scope": args.scope}),
                )
                .await?
        }
    };
    print_skill_result(&result)?;
    println!("audit: {}", session.audit.path().display());
    Ok(())
}
