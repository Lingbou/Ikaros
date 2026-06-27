// SPDX-License-Identifier: GPL-3.0-only

use super::MemoryCandidateCommand;
use super::journal::{append_candidate_journal, append_superseded_journal};
use super::parse::{parse_candidate_status, parse_memory_kind};
use super::projection::refresh_default_projection;
use anyhow::Result;
use ikaros_core::IkarosPaths;
use ikaros_host::memory_candidate_stores;
use ikaros_state::memory::{
    JsonlMemoryCandidateStore, LocalMemoryStore, MemoryCandidate, MemoryCandidateQuery,
    MemoryCandidateStatus, MemoryJournalAction, MemoryRecord, MemoryStore,
};
use serde_json::json;

pub(super) fn memory_candidate_command(
    command: MemoryCandidateCommand,
    paths: &IkarosPaths,
) -> Result<()> {
    let stores = memory_candidate_stores(paths)?;
    let memory_store = stores.memory;
    let candidate_store = stores.candidates;
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
