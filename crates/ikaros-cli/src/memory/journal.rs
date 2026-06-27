// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::IkarosPaths;
use ikaros_state::memory::{
    JsonlMemoryJournal, MemoryCandidate, MemoryJournal, MemoryJournalAction, MemoryJournalEntry,
    MemoryKind, MemoryRecord, WorkingMemoryRecord,
};

pub(super) fn append_projection_rendered_journal(
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

pub(super) fn append_candidate_journal(
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

pub(super) fn append_superseded_journal(
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

pub(super) fn append_working_memory_expired_journal(
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
