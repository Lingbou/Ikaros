// SPDX-License-Identifier: GPL-3.0-only

use crate::{MemoryKind, MemoryQuery, MemoryRecord, MemoryUpdateReport};
use ikaros_core::Result;

use super::{
    MemoryDelegationObservation, MemoryLifecycleReport, MemoryPreCompressInput,
    MemoryPrefetchInput, MemoryProviderDescriptor, MemorySessionSwitch, MemoryTurnRecord,
    MemoryTurnStart,
};

pub trait MemoryProvider: Send + Sync {
    fn descriptor(&self) -> MemoryProviderDescriptor;
    fn turn_start(&self, input: MemoryTurnStart) -> Result<MemoryLifecycleReport>;
    fn prefetch(&self, input: MemoryPrefetchInput) -> Result<Vec<MemoryRecord>>;
    fn sync_turn(&self, turn: MemoryTurnRecord) -> Result<MemoryLifecycleReport>;
    fn pre_compress(&self, input: MemoryPreCompressInput) -> Result<MemoryLifecycleReport>;
    fn session_switch(&self, input: MemorySessionSwitch) -> Result<MemoryLifecycleReport>;
    fn delegation_observation(
        &self,
        input: MemoryDelegationObservation,
    ) -> Result<MemoryLifecycleReport>;
    fn append(&self, record: MemoryRecord) -> Result<MemoryRecord>;
    fn search(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>>;
    fn update(
        &self,
        id: &str,
        content: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Option<MemoryUpdateReport>>;
    fn delete_by_id(&self, id: &str) -> Result<bool>;
    fn delete_scope(&self, query_kind: Option<MemoryKind>, scope: &str) -> Result<usize>;
}
