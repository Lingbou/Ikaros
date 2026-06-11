// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_skill_result, session_and_registry};
use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_memory::MemoryProviderRegistry;
use serde_json::json;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum MemoryCommand {
    Add(MemoryAdd),
    List(MemoryList),
    Search(MemorySearch),
    Update(MemoryUpdate),
    Delete(MemoryDelete),
    Provider {
        #[command(subcommand)]
        command: MemoryProviderCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum MemoryProviderCommand {
    List,
    Active,
    Show(MemoryProviderShow),
}

#[derive(Debug, Args)]
pub(crate) struct MemoryAdd {
    content: String,
    #[arg(long, default_value = "project")]
    kind: String,
    #[arg(long, default_value = "default")]
    scope: String,
    #[arg(long = "tag")]
    tags: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct MemoryList {
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    scope: Option<String>,
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Debug, Args)]
pub(crate) struct MemorySearch {
    query: String,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    scope: Option<String>,
    #[arg(long, default_value_t = 5)]
    limit: usize,
}

#[derive(Debug, Args)]
pub(crate) struct MemoryUpdate {
    id: String,
    #[arg(long)]
    content: Option<String>,
    #[arg(long = "tag")]
    tags: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct MemoryDelete {
    #[arg(long, conflicts_with = "scope")]
    id: Option<String>,
    #[arg(long)]
    scope: Option<String>,
    #[arg(long)]
    kind: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct MemoryProviderShow {
    id: String,
}

pub(crate) async fn memory_command(
    command: MemoryCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    if let MemoryCommand::Provider { command } = command {
        return memory_provider_command(command, paths);
    }

    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = match command {
        MemoryCommand::Add(args) => {
            session
                .execute_skill(
                    &registry,
                    "memory_append",
                    json!({
                        "kind": args.kind,
                        "scope": args.scope,
                        "content": args.content,
                        "tags": args.tags,
                    }),
                )
                .await?
        }
        MemoryCommand::List(args) => {
            let mut input = json!({"limit": args.limit});
            if let Some(kind) = args.kind {
                input["kind"] = json!(kind);
            }
            if let Some(scope) = args.scope {
                input["scope"] = json!(scope);
            }
            session
                .execute_skill(&registry, "memory_search", input)
                .await?
        }
        MemoryCommand::Search(args) => {
            let mut input = json!({"query": args.query, "limit": args.limit});
            if let Some(kind) = args.kind {
                input["kind"] = json!(kind);
            }
            if let Some(scope) = args.scope {
                input["scope"] = json!(scope);
            }
            session
                .execute_skill(&registry, "memory_search", input)
                .await?
        }
        MemoryCommand::Update(args) => {
            let mut input = json!({"id": args.id});
            if let Some(content) = args.content {
                input["content"] = json!(content);
            }
            if !args.tags.is_empty() {
                input["tags"] = json!(args.tags);
            }
            session
                .execute_skill(&registry, "memory_update", input)
                .await?
        }
        MemoryCommand::Delete(args) => {
            let mut input = json!({});
            if let Some(id) = args.id {
                input["id"] = json!(id);
            }
            if let Some(scope) = args.scope {
                input["scope"] = json!(scope);
            }
            if let Some(kind) = args.kind {
                input["kind"] = json!(kind);
            }
            session
                .execute_skill(&registry, "memory_delete", input)
                .await?
        }
        MemoryCommand::Provider { .. } => unreachable!("provider commands return before session"),
    };
    print_skill_result(&result)?;
    println!("audit: {}", session.audit.path().display());
    Ok(())
}

fn memory_provider_command(command: MemoryProviderCommand, paths: &IkarosPaths) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let registry = MemoryProviderRegistry::from_config(
        &paths.memory_dir,
        &config.memory.backend,
        &config.memory.external_providers,
    )?;
    match command {
        MemoryProviderCommand::List => {
            println!("{}", serde_json::to_string_pretty(&registry)?);
        }
        MemoryProviderCommand::Active => {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "local": &registry.active_local,
                    "external": registry.active_external(),
                    "issues": &registry.issues,
                }))?
            );
        }
        MemoryProviderCommand::Show(args) => {
            if registry.active_local.id == args.id {
                println!("{}", serde_json::to_string_pretty(&registry.active_local)?);
                return Ok(());
            }
            let provider = registry
                .external
                .iter()
                .find(|provider| provider.id == args.id)
                .ok_or_else(|| anyhow::anyhow!("memory provider not found: {}", args.id))?;
            println!("{}", serde_json::to_string_pretty(provider)?);
        }
    }
    Ok(())
}
