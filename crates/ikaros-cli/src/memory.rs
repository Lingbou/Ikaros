// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_skill_result, session_and_registry};
use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_core::{IkarosConfig, IkarosPaths};
use ikaros_memory::{
    JsonlMemoryCandidateStore, JsonlMemoryJournal, JsonlWorkingMemoryStore, LocalMemoryStore,
    MemoryCandidate, MemoryCandidateQuery, MemoryCandidateStatus, MemoryJournal,
    MemoryJournalAction, MemoryJournalEntry, MemoryKind, MemoryProjectionFileStore,
    MemoryProjectionInput, MemoryProviderRegistry, MemoryQuery, MemoryRecord, MemoryStore,
    ProjectionRenderer, WorkingMemoryQuery, WorkingMemoryRecord,
};
use serde_json::json;
use std::path::Path;

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
        | MemoryCommand::Working { .. } => {
            unreachable!("local memory maintenance commands return before session")
        }
        MemoryCommand::Provider { .. } => unreachable!("provider commands return before session"),
    };
    print_skill_result(&result)?;
    println!("audit: {}", session.audit.path().display());
    Ok(())
}

fn memory_working_command(command: MemoryWorkingCommand, paths: &IkarosPaths) -> Result<()> {
    let store = JsonlWorkingMemoryStore::new(&paths.memory_dir);
    match command {
        MemoryWorkingCommand::List(args) => {
            let kind = args.kind.as_deref().map(parse_memory_kind).transpose()?;
            let records = store.list(WorkingMemoryQuery {
                session_id: args.session,
                kind,
                scope: args.scope,
                include_expired: args.include_expired,
                limit: Some(args.limit),
            })?;
            println!("{}", serde_json::to_string_pretty(&records)?);
        }
        MemoryWorkingCommand::Prune => {
            let expired = store.prune_expired()?;
            for record in &expired {
                append_working_memory_expired_journal(paths, record)?;
            }
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "summary": "working memory pruned",
                    "expired_count": expired.len(),
                    "expired": expired,
                }))?
            );
        }
    }
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

fn memory_projection_command(command: MemoryProjectionCommand, paths: &IkarosPaths) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let store = LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)?;
    let file_store = MemoryProjectionFileStore::new(&paths.memory_dir);
    match command {
        MemoryProjectionCommand::Render(args) => {
            let projection = render_projection(&store, &args)?;
            let written = file_store.write(&projection, args.scope.as_deref())?;
            append_projection_rendered_journal(paths, args.scope.as_deref())?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "summary": "projection rendered",
                    "directory": file_store.dir(),
                    "files": written,
                }))?
            );
        }
        MemoryProjectionCommand::Show(args) => {
            let projection = file_store.read(args.scope.as_deref())?;
            println!("{}", projection.user.trim_end());
            println!();
            println!("{}", projection.project.trim_end());
            println!();
            println!("{}", projection.general.trim_end());
        }
    }
    Ok(())
}

fn memory_candidate_command(command: MemoryCandidateCommand, paths: &IkarosPaths) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    let memory_store = LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)?;
    let candidate_store = JsonlMemoryCandidateStore::new(&paths.memory_dir);
    match command {
        MemoryCandidateCommand::List(args) => {
            let kind = args.kind.as_deref().map(parse_memory_kind).transpose()?;
            let status = args
                .status
                .as_deref()
                .map(parse_candidate_status)
                .transpose()?;
            let candidates = candidate_store.list(MemoryCandidateQuery {
                status,
                kind,
                scope: args.scope,
                limit: Some(args.limit),
            })?;
            println!("{}", serde_json::to_string_pretty(&candidates)?);
        }
        MemoryCandidateCommand::Accept(args) => {
            let candidate = find_candidate(&candidate_store, &args.id)?;
            let (record, superseded) =
                accept_candidate(&memory_store, &candidate, args.supersedes.as_deref())?;
            let reviewed = candidate_store.set_status(
                &args.id,
                MemoryCandidateStatus::Accepted,
                args.reason,
            )?;
            append_candidate_journal(paths, &candidate, MemoryJournalAction::CandidateAccepted)?;
            if let Some(superseded) = &superseded {
                append_superseded_journal(paths, superseded, &record)?;
            }
            refresh_default_projection(&memory_store, paths, &candidate.scope)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "status": reviewed.map(|candidate| candidate.status),
                    "candidate_id": candidate.id,
                    "memory_id": record.id,
                    "kind": record.kind,
                    "scope": record.scope,
                }))?
            );
        }
        MemoryCandidateCommand::Reject(args) => {
            let reviewed = candidate_store.set_status(
                &args.id,
                MemoryCandidateStatus::Rejected,
                args.reason,
            )?;
            if let Some(candidate) = &reviewed {
                append_candidate_journal(paths, candidate, MemoryJournalAction::CandidateRejected)?;
            }
            println!("{}", serde_json::to_string_pretty(&reviewed)?);
        }
    }
    Ok(())
}

fn render_projection(
    store: &LocalMemoryStore,
    args: &MemoryProjectionArgs,
) -> ikaros_core::Result<ikaros_memory::MemoryProjection> {
    let records = store.list(MemoryQuery {
        limit: Some(usize::MAX),
        ..MemoryQuery::default()
    })?;
    ProjectionRenderer::default().render(MemoryProjectionInput {
        user_scope: args.user_scope.clone(),
        project_scope: args.scope.clone(),
        perspective: None,
        records,
    })
}

fn refresh_default_projection(
    store: &LocalMemoryStore,
    paths: &IkarosPaths,
    project_scope: &str,
) -> Result<()> {
    let args = MemoryProjectionArgs {
        user_scope: "default".into(),
        scope: Some(project_scope.to_owned()),
    };
    let projection = render_projection(store, &args)?;
    MemoryProjectionFileStore::new(&paths.memory_dir).write(&projection, Some(project_scope))?;
    Ok(())
}

fn append_projection_rendered_journal(
    paths: &IkarosPaths,
    project_scope: Option<&str>,
) -> ikaros_core::Result<()> {
    let scope = project_scope.unwrap_or("default");
    let entry = MemoryJournalEntry::new(
        MemoryJournalAction::ProjectionRendered,
        "projection_rendered",
    )?
    .with_scope(Some(MemoryKind::Project), scope)?;
    JsonlMemoryJournal::new(&paths.memory_dir).append(entry)?;
    Ok(())
}

fn append_candidate_journal(
    paths: &IkarosPaths,
    candidate: &MemoryCandidate,
    action: MemoryJournalAction,
) -> ikaros_core::Result<()> {
    let reason = match action {
        MemoryJournalAction::CandidateAccepted => "candidate_accepted",
        MemoryJournalAction::CandidateRejected => "candidate_rejected",
        _ => "candidate_reviewed",
    };
    let mut entry = MemoryJournalEntry::new(action, reason)?.with_memory(
        candidate.id.clone(),
        candidate.kind.clone(),
        candidate.scope.clone(),
    )?;
    if let Some(source_ref) = candidate.source_ref.clone() {
        entry = entry.with_source_ref(source_ref)?;
    }
    JsonlMemoryJournal::new(&paths.memory_dir).append(entry)?;
    Ok(())
}

fn append_superseded_journal(
    paths: &IkarosPaths,
    superseded: &MemoryRecord,
    replacement: &MemoryRecord,
) -> ikaros_core::Result<()> {
    let mut entry = MemoryJournalEntry::new(MemoryJournalAction::Superseded, "superseded")?
        .with_memory(
            superseded.id.clone(),
            superseded.kind.clone(),
            superseded.scope.clone(),
        )?;
    if let Some(source_ref) = replacement.source_ref.clone() {
        entry = entry.with_source_ref(source_ref)?;
    }
    JsonlMemoryJournal::new(&paths.memory_dir).append(entry)?;
    Ok(())
}

fn append_working_memory_expired_journal(
    paths: &IkarosPaths,
    record: &WorkingMemoryRecord,
) -> ikaros_core::Result<()> {
    let mut entry = MemoryJournalEntry::new(
        MemoryJournalAction::WorkingMemoryExpired,
        "working_memory_expired",
    )?
    .with_memory(record.id.clone(), record.kind.clone(), record.scope.clone())?;
    if let Some(source_ref) = record.source_ref.clone() {
        entry = entry.with_source_ref(source_ref)?;
    }
    JsonlMemoryJournal::new(&paths.memory_dir).append(entry)?;
    Ok(())
}

fn find_candidate(
    store: &JsonlMemoryCandidateStore,
    id: &str,
) -> ikaros_core::Result<MemoryCandidate> {
    store
        .list(MemoryCandidateQuery {
            limit: Some(usize::MAX),
            ..MemoryCandidateQuery::default()
        })?
        .into_iter()
        .find(|candidate| candidate.id == id)
        .ok_or_else(|| {
            ikaros_core::IkarosError::Message(format!("memory candidate not found: {id}"))
        })
}

fn accept_candidate(
    store: &LocalMemoryStore,
    candidate: &MemoryCandidate,
    supersedes: Option<&str>,
) -> ikaros_core::Result<(MemoryRecord, Option<MemoryRecord>)> {
    let mut record = MemoryRecord::new(
        candidate.kind.clone(),
        candidate.scope.clone(),
        candidate.content.clone(),
    )?
    .with_tags(vec!["candidate-accepted".into()])
    .with_source("memory_candidate");
    if let Some(source_ref) = candidate.source_ref.clone() {
        record = record.with_source_ref(source_ref);
    }
    record.confidence = Some(candidate.confidence);
    if let Some(superseded_id) = supersedes {
        return store.supersede(superseded_id, record)?.map_or_else(
            || {
                Err(ikaros_core::IkarosError::Message(format!(
                    "memory to supersede not found: {superseded_id}"
                )))
            },
            |(superseded, active)| Ok((active, Some(superseded))),
        );
    }
    store.append(record).map(|record| (record, None))
}

fn parse_candidate_status(status: &str) -> ikaros_core::Result<MemoryCandidateStatus> {
    match status.to_ascii_lowercase().as_str() {
        "pending" => Ok(MemoryCandidateStatus::Pending),
        "accepted" => Ok(MemoryCandidateStatus::Accepted),
        "rejected" => Ok(MemoryCandidateStatus::Rejected),
        "expired" => Ok(MemoryCandidateStatus::Expired),
        other => Err(ikaros_core::IkarosError::Message(format!(
            "unsupported memory candidate status: {other}"
        ))),
    }
}

fn parse_memory_kind(kind: &str) -> ikaros_core::Result<MemoryKind> {
    match kind.to_ascii_lowercase().as_str() {
        "user" => Ok(MemoryKind::User),
        "project" => Ok(MemoryKind::Project),
        "task" => Ok(MemoryKind::Task),
        "persona" => Ok(MemoryKind::Persona),
        "relationship" => Ok(MemoryKind::Relationship),
        "knowledge" => Ok(MemoryKind::Knowledge),
        other => Err(ikaros_core::IkarosError::Message(format!(
            "unsupported memory kind: {other}"
        ))),
    }
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
