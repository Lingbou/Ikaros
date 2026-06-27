// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    MemoryKind, MemoryPerspective, MemoryQuery, MemoryRecord, MemoryRef, MemoryStore,
    MemoryUpdateReport,
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
                perspective_json TEXT,
                content TEXT NOT NULL,
                tags_json TEXT NOT NULL,
                source TEXT,
                source_ref_json TEXT,
                confidence REAL,
                sensitive INTEGER NOT NULL DEFAULT 0,
                active INTEGER NOT NULL DEFAULT 1,
                supersedes_json TEXT NOT NULL DEFAULT '[]',
                superseded_by TEXT,
                valid_from TEXT,
                valid_until TEXT
            );
            CREATE INDEX IF NOT EXISTS memories_kind_scope_idx ON memories(kind, scope);
            CREATE INDEX IF NOT EXISTS memories_created_at_idx ON memories(created_at);
            "#,
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        require_memory_columns(
            conn,
            &self.path,
            &[
                "id",
                "created_at",
                "updated_at",
                "kind",
                "scope",
                "perspective_json",
                "content",
                "tags_json",
                "source",
                "source_ref_json",
                "confidence",
                "sensitive",
                "active",
                "supersedes_json",
                "superseded_by",
                "valid_from",
                "valid_until",
            ],
        )?;
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<MemoryRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, created_at, updated_at, kind, scope, perspective_json, content, tags_json, source, source_ref_json, confidence, sensitive, active, supersedes_json, superseded_by, valid_from, valid_until FROM memories",
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
        let rows = stmt
            .query_map([], |row| {
                let tags_json: String = row.get(7)?;
                let tags = serde_json::from_str::<Vec<String>>(&tags_json).unwrap_or_default();
                let supersedes_json: String = row.get(13)?;
                let supersedes =
                    serde_json::from_str::<Vec<String>>(&supersedes_json).unwrap_or_default();
                let kind_raw: String = row.get(3)?;
                Ok(MemoryRecord {
                    id: row.get(0)?,
                    created_at: row.get(1)?,
                    updated_at: row.get(2)?,
                    active: row.get::<_, i64>(12)? != 0,
                    supersedes,
                    superseded_by: row.get(14)?,
                    valid_from: row.get(15)?,
                    valid_until: row.get(16)?,
                    kind: memory_kind_from_str(&kind_raw).unwrap_or(MemoryKind::Knowledge),
                    scope: row.get(4)?,
                    perspective: perspective_from_json(row.get::<_, Option<String>>(5)?),
                    content: row.get(6)?,
                    tags,
                    source: row.get(8)?,
                    source_ref: source_ref_from_json(row.get::<_, Option<String>>(9)?),
                    confidence: row.get::<_, Option<f64>>(10)?.map(|value| value as f32),
                    sensitive: row.get::<_, i64>(11)? != 0,
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
        let supersedes_json = serde_json::to_string(&record.supersedes)?;
        let perspective_json = record
            .perspective
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let source_ref_json = record
            .source_ref
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        conn.execute(
            "INSERT INTO memories (id, created_at, updated_at, kind, scope, perspective_json, content, tags_json, source, source_ref_json, confidence, sensitive, active, supersedes_json, superseded_by, valid_from, valid_until)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                record.id,
                record.created_at,
                record.updated_at,
                memory_kind_to_str(&record.kind),
                record.scope,
                perspective_json,
                record.content,
                tags_json,
                record.source,
                source_ref_json,
                record.confidence.map(f64::from),
                i64::from(record.sensitive),
                i64::from(record.active),
                supersedes_json,
                record.superseded_by,
                record.valid_from,
                record.valid_until,
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
    ) -> Result<Option<MemoryUpdateReport>> {
        if let Some(content) = &content {
            reject_secret_like(content, "memory content")?;
        }
        let current = self.read_all()?.into_iter().find(|record| record.id == id);
        let Some(mut record) = current else {
            return Ok(None);
        };
        let before = record.clone();
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
        Ok(Some(MemoryUpdateReport::from_before_after(&before, record)))
    }

    fn supersede(
        &self,
        old_id: &str,
        mut replacement: MemoryRecord,
    ) -> Result<Option<(MemoryRecord, MemoryRecord)>> {
        reject_secret_like(old_id, "memory superseded id")?;
        if replacement.id == old_id {
            return Err(IkarosError::Message(
                "replacement memory cannot supersede itself".into(),
            ));
        }
        reject_secret_like(&replacement.content, "memory content")?;
        replacement.validate_metadata()?;
        if replacement.sensitive || contains_secret_like(&replacement.content) {
            return Err(IkarosError::SecretRejected("memory content".into()));
        }

        let Some(mut superseded) = self
            .read_all()?
            .into_iter()
            .find(|record| record.id == old_id)
        else {
            return Ok(None);
        };
        let now = now_rfc3339()?;
        replacement.active = true;
        replacement.updated_at = None;
        replacement.valid_from.get_or_insert_with(|| now.clone());
        if !replacement.supersedes.iter().any(|id| id == old_id) {
            replacement.supersedes.push(old_id.to_owned());
        }
        replacement.validate_metadata()?;

        superseded.active = false;
        superseded.updated_at = Some(now.clone());
        superseded.valid_until = Some(now);
        superseded.superseded_by = Some(replacement.id.clone());
        superseded.validate_metadata()?;

        let conn = self.open()?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|source| sqlite_error(&self.path, source))?;
        tx.execute(
            "UPDATE memories SET updated_at = ?1, active = ?2, superseded_by = ?3, valid_until = ?4 WHERE id = ?5",
            params![
                &superseded.updated_at,
                i64::from(superseded.active),
                &superseded.superseded_by,
                &superseded.valid_until,
                old_id,
            ],
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        let tags_json = serde_json::to_string(&replacement.tags)?;
        let supersedes_json = serde_json::to_string(&replacement.supersedes)?;
        let perspective_json = replacement
            .perspective
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        let source_ref_json = replacement
            .source_ref
            .as_ref()
            .map(serde_json::to_string)
            .transpose()?;
        tx.execute(
            "INSERT INTO memories (id, created_at, updated_at, kind, scope, perspective_json, content, tags_json, source, source_ref_json, confidence, sensitive, active, supersedes_json, superseded_by, valid_from, valid_until)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                replacement.id,
                replacement.created_at,
                replacement.updated_at,
                memory_kind_to_str(&replacement.kind),
                replacement.scope,
                perspective_json,
                replacement.content,
                tags_json,
                replacement.source,
                source_ref_json,
                replacement.confidence.map(f64::from),
                i64::from(replacement.sensitive),
                i64::from(replacement.active),
                supersedes_json,
                replacement.superseded_by,
                replacement.valid_from,
                replacement.valid_until,
            ],
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        tx.commit()
            .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(Some((superseded, replacement)))
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

fn require_memory_columns(conn: &Connection, path: &Path, required: &[&str]) -> Result<()> {
    let mut stmt = conn
        .prepare("PRAGMA table_info(memories)")
        .map_err(|source| sqlite_error(path, source))?;
    let existing = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|source| sqlite_error(path, source))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|source| sqlite_error(path, source))?;
    for column in required {
        if !existing.iter().any(|name| name == column) {
            return Err(IkarosError::Message(format!(
                "memory SQLite store at {} is missing required column {column}; delete the store and rebuild memory",
                path.display()
            )));
        }
    }
    Ok(())
}

fn source_ref_from_json(value: Option<String>) -> Option<MemoryRef> {
    value.and_then(|value| serde_json::from_str(&value).ok())
}

fn perspective_from_json(value: Option<String>) -> Option<MemoryPerspective> {
    value.and_then(|value| serde_json::from_str(&value).ok())
}
