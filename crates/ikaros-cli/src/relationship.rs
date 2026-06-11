// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_approval_hint, print_skill_result};
use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_core::IkarosPaths;
use ikaros_runtime::{
    forget_relationship_note_by_id, forget_relationship_scope, relationship_snapshot,
    remember_relationship_note,
};
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum RelationshipCommand {
    Show(RelationshipShow),
    Remember(RelationshipRemember),
    Forget(RelationshipForget),
}

#[derive(Debug, Args)]
pub(crate) struct RelationshipShow {
    #[arg(long)]
    scope: Option<String>,
    #[arg(long, default_value_t = 10)]
    limit: usize,
}

#[derive(Debug, Args)]
pub(crate) struct RelationshipRemember {
    note: String,
    #[arg(long, default_value = "user")]
    scope: String,
    #[arg(long = "tag")]
    tags: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct RelationshipForget {
    #[arg(long, conflicts_with = "scope")]
    id: Option<String>,
    #[arg(long)]
    scope: Option<String>,
}

pub(crate) async fn relationship_command(
    command: RelationshipCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        RelationshipCommand::Show(args) => {
            let snapshot = relationship_snapshot(
                paths,
                workspace,
                agent_override,
                args.scope.as_deref(),
                args.limit,
            )
            .await?;
            println!(
                "scope: {}",
                snapshot
                    .scope
                    .as_deref()
                    .unwrap_or("all relationship scopes")
            );
            println!("notes: {}", snapshot.notes.len());
            for note in &snapshot.notes {
                let tags = if note.tags.is_empty() {
                    String::new()
                } else {
                    format!(" tags={}", note.tags.join(","))
                };
                println!("- {} [{}] {}{}", note.id, note.scope, note.content, tags);
            }
            println!("audit: {}", snapshot.audit_path.display());
        }
        RelationshipCommand::Remember(args) => {
            let report = remember_relationship_note(
                paths,
                workspace,
                agent_override,
                &args.scope,
                &args.note,
                args.tags,
            )
            .await?;
            print_skill_result(&report.result)?;
            print_approval_hint(&report.result);
            println!("audit: {}", report.audit_path.display());
        }
        RelationshipCommand::Forget(args) => {
            let report = if let Some(id) = args.id {
                forget_relationship_note_by_id(paths, workspace, agent_override, &id).await?
            } else if let Some(scope) = args.scope {
                forget_relationship_scope(paths, workspace, agent_override, &scope).await?
            } else {
                anyhow::bail!("relationship forget requires --id <id> or --scope <scope>");
            };
            print_skill_result(&report.result)?;
            print_approval_hint(&report.result);
            println!("audit: {}", report.audit_path.display());
        }
    }
    Ok(())
}
