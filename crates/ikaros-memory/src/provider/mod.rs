// SPDX-License-Identifier: GPL-3.0-only

mod descriptor;
mod lifecycle;
mod local;
mod noop;
mod registry;
mod traits;

pub use descriptor::{MemoryProviderDescriptor, MemoryProviderKind, MemoryProviderState};
pub use lifecycle::{
    MemoryDelegationObservation, MemoryLifecycleRecordRef, MemoryLifecycleReport,
    MemoryPreCompressInput, MemoryPrefetchInput, MemorySessionSwitch, MemoryTurnRecord,
    MemoryTurnStart,
};
pub use noop::NoopMemoryProvider;
pub use registry::MemoryProviderRegistry;
pub use traits::MemoryProvider;
