// SPDX-License-Identifier: GPL-3.0-only

mod candidate;
mod journal;
mod parse;
mod projection;
mod provider;
mod supersession;
mod working;

use crate::{print_skill_result, session_and_registry};
use anyhow::Result;
use candidate::memory_candidate_command;
use clap::{Args, Subcommand};
use ikaros_core::IkarosPaths;
use projection::memory_projection_command;
use provider::memory_provider_command;
use serde_json::json;
use std::path::Path;
use supersession::memory_supersession_command;
use working::memory_working_command;

#[derive(Debug, Subcommand)]
pub(crate) enum MemoryCommand {
    Add(MemoryAdd),
    List(MemoryList),
    Search(MemorySearch),
    Update(MemoryUpdate),
    Delete(MemoryDelete),
    Projection {
        #[command(subcommand)]
        command: MemoryProjectionCommand,
    },
    Candidate {
        #[command(subcommand)]
        command: MemoryCandidateCommand,
    },
    Supersession(MemorySupersession),
    Working {
        #[command(subcommand)]
        command: MemoryWorkingCommand,
    },
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

#[derive(Debug, Subcommand)]
pub(crate) enum MemoryProjectionCommand {
    Render(MemoryProjectionArgs),
    Show(MemoryProjectionArgs),
}

#[derive(Debug, Args)]
pub(crate) struct MemoryProjectionArgs {
    #[arg(long = "user-scope", default_value = "default")]
    user_scope: String,
    #[arg(long = "scope")]
    scope: Option<String>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum MemoryCandidateCommand {
    List(MemoryCandidateList),
    Accept(MemoryCandidateReview),
    Reject(MemoryCandidateReview),
}

#[derive(Debug, Subcommand)]
pub(crate) enum MemoryWorkingCommand {
    List(MemoryWorkingList),
    Prune,
}

#[derive(Debug, Args)]
pub(crate) struct MemoryWorkingList {
    #[arg(long)]
    session: Option<String>,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    scope: Option<String>,
    #[arg(long)]
    include_expired: bool,
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Debug, Args)]
pub(crate) struct MemoryCandidateList {
    #[arg(long)]
    status: Option<String>,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    scope: Option<String>,
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

#[derive(Debug, Args)]
pub(crate) struct MemoryCandidateReview {
    id: String,
    #[arg(long, default_value = "manual review")]
    reason: String,
    #[arg(long)]
    supersedes: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct MemorySupersession {
    id: String,
}

#[derive(Debug, Args)]
pub(crate) struct MemoryAdd {
    content: String,
    #[arg(long, default_value = "project")]
    kind: String,
    #[arg(long, default_value = "default")]
    scope: String,
    #[arg(long)]
    observer: Option<String>,
    #[arg(long)]
    subject: Option<String>,
    #[arg(long = "tag")]
    tags: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct MemoryList {
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    scope: Option<String>,
    #[arg(long)]
    observer: Option<String>,
    #[arg(long)]
    subject: Option<String>,
    #[arg(long, default_value_t = 20)]
    limit: usize,
    #[arg(long)]
    include_inactive: bool,
}

#[derive(Debug, Args)]
pub(crate) struct MemorySearch {
    query: String,
    #[arg(long)]
    kind: Option<String>,
    #[arg(long)]
    scope: Option<String>,
    #[arg(long)]
    observer: Option<String>,
    #[arg(long)]
    subject: Option<String>,
    #[arg(long, default_value_t = 5)]
    limit: usize,
    #[arg(long)]
    include_inactive: bool,
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
    if let MemoryCommand::Projection { command } = command {
        return memory_projection_command(command, paths);
    }
    if let MemoryCommand::Candidate { command } = command {
        return memory_candidate_command(command, paths);
    }
    if let MemoryCommand::Supersession(args) = command {
        return memory_supersession_command(args, paths);
    }
    if let MemoryCommand::Working { command } = command {
        return memory_working_command(command, paths);
    }

    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let result = match command {
        MemoryCommand::Add(args) => {
            let mut input = json!({
                "kind": args.kind,
                "scope": args.scope,
                "content": args.content,
                "tags": args.tags,
            });
            apply_perspective_fields(&mut input, args.observer, args.subject)?;
            session
                .execute_skill(&registry, "memory_append", input)
                .await?
        }
        MemoryCommand::List(args) => {
            let mut input = json!({"limit": args.limit});
            if args.include_inactive {
                input["include_inactive"] = json!(true);
            }
            if let Some(kind) = args.kind {
                input["kind"] = json!(kind);
            }
            if let Some(scope) = args.scope {
                input["scope"] = json!(scope);
            }
            apply_perspective_fields(&mut input, args.observer, args.subject)?;
            session
                .execute_skill(&registry, "memory_search", input)
                .await?
        }
        MemoryCommand::Search(args) => {
            let mut input = json!({"query": args.query, "limit": args.limit});
            if args.include_inactive {
                input["include_inactive"] = json!(true);
            }
            if let Some(kind) = args.kind {
                input["kind"] = json!(kind);
            }
            if let Some(scope) = args.scope {
                input["scope"] = json!(scope);
            }
            apply_perspective_fields(&mut input, args.observer, args.subject)?;
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
        MemoryCommand::Projection { .. }
        | MemoryCommand::Candidate { .. }
        | MemoryCommand::Supersession(..)
        | MemoryCommand::Working { .. } => {
            unreachable!("local memory maintenance commands return before session")
        }
        MemoryCommand::Provider { .. } => unreachable!("provider commands return before session"),
    };
    print_skill_result(&result)?;
    println!("audit: {}", session.audit.path().display());
    Ok(())
}

fn apply_perspective_fields(
    input: &mut serde_json::Value,
    observer: Option<String>,
    subject: Option<String>,
) -> Result<()> {
    match (observer, subject) {
        (Some(observer), Some(subject)) => {
            input["observer"] = json!(observer);
            input["subject"] = json!(subject);
            Ok(())
        }
        (None, None) => Ok(()),
        _ => Err(anyhow::anyhow!(
            "--observer and --subject must be provided together"
        )),
    }
}
