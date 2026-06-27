// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::Result;
use std::path::Path;

use super::{MemoryKind, MemoryQuery, MemoryRecord, MemoryUpdateReport};

pub trait MemoryStore {
    fn append(&self, record: MemoryRecord) -> Result<MemoryRecord>;
    fn list(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>>;
    fn search(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>>;
    fn update(
        &self,
        id: &str,
        content: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Option<MemoryUpdateReport>>;
    fn supersede(
        &self,
        old_id: &str,
        replacement: MemoryRecord,
    ) -> Result<Option<(MemoryRecord, MemoryRecord)>>;
    fn delete_by_id(&self, id: &str) -> Result<bool>;
    fn delete_scope(&self, kind: Option<MemoryKind>, scope: &str) -> Result<usize>;
    fn path(&self) -> &Path;
    fn backend_name(&self) -> &'static str;
}
