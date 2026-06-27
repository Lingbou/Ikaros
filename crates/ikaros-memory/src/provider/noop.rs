// SPDX-License-Identifier: GPL-3.0-only

use crate::{MemoryKind, MemoryQuery, MemoryRecord, MemoryUpdateReport};
use ikaros_core::Result;

use super::{
    MemoryDelegationObservation, MemoryLifecycleReport, MemoryPreCompressInput,
    MemoryPrefetchInput, MemoryProvider, MemoryProviderDescriptor, MemoryProviderKind,
    MemoryProviderState, MemorySessionSwitch, MemoryTurnRecord, MemoryTurnStart,
};

#[derive(Debug, Clone, Default)]
pub struct NoopMemoryProvider;

impl MemoryProvider for NoopMemoryProvider {
    fn descriptor(&self) -> MemoryProviderDescriptor {
        MemoryProviderDescriptor {
            id: "noop".into(),
            kind: MemoryProviderKind::BuiltinLocal,
            backend: "noop".into(),
            state: MemoryProviderState::Disabled,
            path: None,
            endpoint: None,
            api_key_configured: false,
            notes: vec!["explicit noop memory provider".into()],
        }
    }

    fn turn_start(&self, _input: MemoryTurnStart) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("turn_start"))
    }

    fn prefetch(&self, _input: MemoryPrefetchInput) -> Result<Vec<MemoryRecord>> {
        Ok(Vec::new())
    }

    fn sync_turn(&self, _turn: MemoryTurnRecord) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("sync_turn"))
    }

    fn pre_compress(&self, _input: MemoryPreCompressInput) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("pre_compress"))
    }

    fn session_switch(&self, _input: MemorySessionSwitch) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("session_switch"))
    }

    fn delegation_observation(
        &self,
        _input: MemoryDelegationObservation,
    ) -> Result<MemoryLifecycleReport> {
        Ok(MemoryLifecycleReport::noop("delegation_observation"))
    }

    fn append(&self, record: MemoryRecord) -> Result<MemoryRecord> {
        Ok(record)
    }

    fn search(&self, _query: MemoryQuery) -> Result<Vec<MemoryRecord>> {
        Ok(Vec::new())
    }

    fn update(
        &self,
        _id: &str,
        _content: Option<String>,
        _tags: Option<Vec<String>>,
    ) -> Result<Option<MemoryUpdateReport>> {
        Ok(None)
    }

    fn delete_by_id(&self, _id: &str) -> Result<bool> {
        Ok(false)
    }

    fn delete_scope(&self, _query_kind: Option<MemoryKind>, _scope: &str) -> Result<usize> {
        Ok(0)
    }
}
