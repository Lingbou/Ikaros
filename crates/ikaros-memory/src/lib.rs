// SPDX-License-Identifier: GPL-3.0-only
//! Local-first memory storage for Ikaros.

mod common;
mod jsonl;
mod local;
mod provider;
mod sqlite;
mod types;

pub use jsonl::JsonlMemoryStore;
pub use local::LocalMemoryStore;
pub use provider::{
    MemoryDelegationObservation, MemoryLifecycleReport, MemoryPreCompressInput,
    MemoryPrefetchInput, MemoryProvider, MemoryProviderDescriptor, MemoryProviderKind,
    MemoryProviderRegistry, MemoryProviderState, MemorySessionSwitch, MemoryTurnRecord,
    MemoryTurnStart,
};
pub use sqlite::SqliteMemoryStore;
pub use types::{MemoryKind, MemoryQuery, MemoryRecord, MemoryStore};

#[cfg(test)]
mod tests;
