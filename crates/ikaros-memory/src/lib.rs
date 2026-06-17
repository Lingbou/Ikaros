// SPDX-License-Identifier: GPL-3.0-only
//! Local-first memory storage for Ikaros.

mod common;
mod journal;
mod jsonl;
mod local;
mod policy;
mod provider;
mod relationship;
mod sqlite;
mod types;

pub use journal::{
    JsonlMemoryJournal, MemoryJournal, MemoryJournalAction, MemoryJournalEntry, MemoryPolicy,
    MemoryScore,
};
pub use jsonl::JsonlMemoryStore;
pub use local::LocalMemoryStore;
pub use policy::{MemoryPolicyDecision, MemoryPolicyEngine, add_policy_tag, has_policy_tag};
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
pub use types::{MemoryKind, MemoryQuery, MemoryRecord, MemoryRef, MemoryStore};

#[cfg(test)]
mod tests;
