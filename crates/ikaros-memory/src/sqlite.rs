// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    MemoryKind, MemoryQuery, MemoryRecord, MemoryStore,
    common::{filter_records, memory_kind_from_str, memory_kind_to_str},
};
use ikaros_core::{IkarosError, Result, contains_secret_like, now_rfc3339, reject_secret_like};
use rusqlite::{Connection, params};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct SqliteMemoryStore {
    path: PathBuf,
}

impl SqliteMemoryStore {
    pub fn new(memory_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: memory_dir.into().join("memory.sqlite"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn open(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let conn =
            Connection::open(&self.path).map_err(|source| sqlite_error(&self.path, source))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|source| sqlite_error(&self.path, source))?;
        self.ensure_schema(&conn)?;
        Ok(conn)
    }

    fn ensure_schema(&self, conn: &Connection) -> Result<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                updated_at TEXT,
                kind TEXT NOT NULL,
                scope TEXT NOT NULL,
                content TEXT NOT NULL,
                tags_json TEXT NOT NULL,
                source TEXT,
                confidence REAL,
                sensitive INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS memories_kind_scope_idx ON memories(kind, scope);
            CREATE INDEX IF NOT EXISTS memories_created_at_idx ON memories(created_at);
            "#,
        )
        .map_err(|source| sqlite_error(&self.path, source))
    }

    fn read_all(&self) -> Result<Vec<MemoryRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, created_at, updated_at, kind, scope, content, tags_json, source, confidence, sensitive FROM memories",
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
        let rows = stmt
            .query_map([], |row| {
                let tags_json: String = row.get(6)?;
                let tags = serde_json::from_str::<Vec<String>>(&tags_json).unwrap_or_default();
                let kind_raw: String = row.get(3)?;
                Ok(MemoryRecord {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    kind: memory_kind_from_str(&kind_raw).unwrap_or(MemoryKind::Knowledge),
                    scope: row.get(4)?,
                    content: row.get(5)?,
                    tags,
                    source: row.get(7)?,
                    confidence: row.get::<_, Option<f64>>(8)?.map(|value| value as f32),
                    sensitive: row.get::<_, i64>(9)? != 0,
                })
            })
            .map_err(|source| sqlite_error(&self.path, source))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|source| sqlite_error(&self.path, source))?);
        }
        Ok(records)
    }
}

impl MemoryStore for SqliteMemoryStore {
    fn append(&self, mut record: MemoryRecord) -> Result<MemoryRecord> {
        reject_secret_like(&record.content, "memory content")?;
        record.validate_metadata()?;
        if record.sensitive || contains_secret_like(&record.content) {
            return Err(IkarosError::SecretRejected("memory content".into()));
        }
        record.updated_at = None;
        let conn = self.open()?;
        let tags_json = serde_json::to_string(&record.tags)?;
        conn.execute(
            "INSERT INTO memories (id, created_at, updated_at, kind, scope, content, tags_json, source, confidence, sensitive)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                record.id,
                record.created_at,
                record.updated_at,
                memory_kind_to_str(&record.kind),
                record.scope,
                record.content,
                tags_json,
                record.source,
                record.confidence.map(f64::from),
                i64::from(record.sensitive),
            ],
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(record)
    }

    fn list(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>> {
        let records = self.read_all()?;
        Ok(filter_records(records, &query))
    }

    fn search(&self, mut query: MemoryQuery) -> Result<Vec<MemoryRecord>> {
        if query.limit.is_none() {
            query.limit = Some(20);
        }
        let records = self.read_all()?;
        Ok(filter_records(records, &query))
    }

    fn update(
        &self,
        id: &str,
        content: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Option<MemoryRecord>> {
        if let Some(content) = &content {
            reject_secret_like(content, "memory content")?;
        }
        let current = self.read_all()?.into_iter().find(|record| record.id == id);
        let Some(mut record) = current else {
            return Ok(None);
        };
        if let Some(content) = content {
            record.content = content;
        }
        if let Some(tags) = tags {
            record.tags = tags;
        }
        reject_secret_like(&record.content, "memory content")?;
        record.validate_metadata()?;
        if record.sensitive || contains_secret_like(&record.content) {
            return Err(IkarosError::SecretRejected("memory content".into()));
        }
        record.updated_at = Some(now_rfc3339()?);
        let conn = self.open()?;
        let tags_json = serde_json::to_string(&record.tags)?;
        conn.execute(
            "UPDATE memories SET updated_at = ?1, content = ?2, tags_json = ?3 WHERE id = ?4",
            params![&record.updated_at, &record.content, &tags_json, id],
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(Some(record))
    }

    fn delete_by_id(&self, id: &str) -> Result<bool> {
        let conn = self.open()?;
        let deleted = conn
            .execute("DELETE FROM memories WHERE id = ?1", params![id])
            .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(deleted > 0)
    }

    fn delete_scope(&self, kind: Option<MemoryKind>, scope: &str) -> Result<usize> {
        let conn = self.open()?;
        let deleted = if let Some(kind) = kind {
            conn.execute(
                "DELETE FROM memories WHERE kind = ?1 AND scope = ?2",
                params![memory_kind_to_str(&kind), scope],
            )
        } else {
            conn.execute("DELETE FROM memories WHERE scope = ?1", params![scope])
        }
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(deleted)
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn backend_name(&self) -> &'static str {
        "sqlite"
    }
}

fn sqlite_error(path: &Path, source: rusqlite::Error) -> IkarosError {
    IkarosError::Message(format!("sqlite error at {}: {source}", path.display()))
}
