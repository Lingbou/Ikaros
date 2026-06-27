// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    JsonlMemoryStore, MemoryKind, MemoryQuery, MemoryRecord, MemoryStore, MemoryUpdateReport,
    SqliteMemoryStore,
};
use ikaros_core::{IkarosError, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum LocalMemoryStore {
    Jsonl(JsonlMemoryStore),
    Sqlite(SqliteMemoryStore),
}

impl LocalMemoryStore {
    pub fn new(memory_dir: impl Into<PathBuf>, backend: &str) -> Result<Self> {
        let memory_dir = memory_dir.into();
        match backend {
            "jsonl" => Ok(Self::Jsonl(JsonlMemoryStore::new(memory_dir))),
            "sqlite" => Ok(Self::Sqlite(SqliteMemoryStore::new(memory_dir))),
            other => Err(IkarosError::Message(format!(
                "unsupported memory backend: {other}"
            ))),
        }
    }

    pub fn memory_dir(&self) -> PathBuf {
        self.path()
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

impl MemoryStore for LocalMemoryStore {
    fn append(&self, record: MemoryRecord) -> Result<MemoryRecord> {
        match self {
            Self::Jsonl(store) => store.append(record),
            Self::Sqlite(store) => store.append(record),
        }
    }

    fn list(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>> {
        match self {
            Self::Jsonl(store) => store.list(query),
            Self::Sqlite(store) => store.list(query),
        }
    }

    fn search(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>> {
        match self {
            Self::Jsonl(store) => store.search(query),
            Self::Sqlite(store) => store.search(query),
        }
    }

    fn update(
        &self,
        id: &str,
        content: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Option<MemoryUpdateReport>> {
        match self {
            Self::Jsonl(store) => store.update(id, content, tags),
            Self::Sqlite(store) => store.update(id, content, tags),
        }
    }

    fn supersede(
        &self,
        old_id: &str,
        replacement: MemoryRecord,
    ) -> Result<Option<(MemoryRecord, MemoryRecord)>> {
        match self {
            Self::Jsonl(store) => store.supersede(old_id, replacement),
            Self::Sqlite(store) => store.supersede(old_id, replacement),
        }
    }

    fn delete_by_id(&self, id: &str) -> Result<bool> {
        match self {
            Self::Jsonl(store) => store.delete_by_id(id),
            Self::Sqlite(store) => store.delete_by_id(id),
        }
    }

    fn delete_scope(&self, kind: Option<MemoryKind>, scope: &str) -> Result<usize> {
        match self {
            Self::Jsonl(store) => store.delete_scope(kind, scope),
            Self::Sqlite(store) => store.delete_scope(kind, scope),
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::Jsonl(store) => store.path(),
            Self::Sqlite(store) => store.path(),
        }
    }

    fn backend_name(&self) -> &'static str {
        match self {
            Self::Jsonl(store) => store.backend_name(),
            Self::Sqlite(store) => store.backend_name(),
        }
    }
}
