// SPDX-License-Identifier: GPL-3.0-only
//! Local-first memory storage for Ikaros.

mod candidate;
mod common;
mod journal;
mod jsonl;
mod local;
mod policy;
mod projection;
mod provider;
mod relationship;
mod sqlite;
mod types;
mod working;

pub use candidate::{
    JsonlMemoryCandidateStore, MemoryCandidate, MemoryCandidateQuery, MemoryCandidateReason,
    MemoryCandidateStatus,
};
pub use journal::{
    JsonlMemoryJournal, MemoryJournal, MemoryJournalAction, MemoryJournalEntry, MemoryPolicy,
    MemoryScore,
};
pub use jsonl::JsonlMemoryStore;
pub use local::LocalMemoryStore;
pub use policy::{MemoryPolicyDecision, MemoryPolicyEngine, add_policy_tag, has_policy_tag};
pub use projection::{
    MemoryProjection, MemoryProjectionFileStore, MemoryProjectionInput, ProjectionRenderer,
};
pub use provider::{
    MemoryDelegationObservation, MemoryLifecycleRecordRef, MemoryLifecycleReport,
    MemoryPreCompressInput, MemoryPrefetchInput, MemoryProvider, MemoryProviderDescriptor,
    MemoryProviderKind, MemoryProviderRegistry, MemoryProviderState, MemorySessionSwitch,
    MemoryTurnRecord, MemoryTurnStart, NoopMemoryProvider,
};
pub use relationship::{
    RelationshipMemoryNote, relationship_context_lines, relationship_notes_from_output,
};
pub use sqlite::SqliteMemoryStore;
pub use types::{
    MemoryChangeReport, MemoryKind, MemoryPerspective, MemoryQuery, MemoryRecord, MemoryRef,
    MemoryStore, MemoryUpdateReport, MemoryUpdateSnapshot,
};
pub use working::{JsonlWorkingMemoryStore, WorkingMemoryQuery, WorkingMemoryRecord};

#[cfg(test)]
mod tests;
