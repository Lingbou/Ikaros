// SPDX-License-Identifier: GPL-3.0-only

use crate::{MemoryKind, MemoryRef};
use ikaros_core::{IkarosError, Result, now_rfc3339, reject_secret_like};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkingMemoryRecord {
    pub id: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    pub session_id: String,
    pub kind: MemoryKind,
    pub scope: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<MemoryRef>,
}

impl WorkingMemoryRecord {
    pub fn new(
        session_id: impl Into<String>,
        kind: MemoryKind,
        scope: impl Into<String>,
        content: impl Into<String>,
        ttl_hours: Option<i64>,
    ) -> Result<Self> {
        let session_id = session_id.into();
        let scope = scope.into();
        let content = content.into();
        reject_secret_like(&session_id, "working memory session id")?;
        reject_secret_like(&scope, "working memory scope")?;
        reject_secret_like(&content, "working memory content")?;
        let expires_at = match ttl_hours {
            Some(hours) if hours > 0 => Some(
                (OffsetDateTime::now_utc() + Duration::hours(hours))
                    .format(&Rfc3339)
                    .map_err(IkarosError::from)?,
            ),
            _ => None,
        };
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            created_at: now_rfc3339()?,
            expires_at,
            session_id,
            kind,
            scope,
            content,
            tags: Vec::new(),
            source_ref: None,
        })
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Result<Self> {
        for tag in &tags {
            reject_secret_like(tag, "working memory tag")?;
        }
        self.tags = tags;
        Ok(self)
    }

    pub fn with_source_ref(mut self, source_ref: MemoryRef) -> Result<Self> {
        source_ref.validate()?;
        self.source_ref = Some(source_ref);
        Ok(self)
    }

    pub fn is_expired(&self) -> bool {
        self.expires_at
            .as_deref()
            .and_then(|expires_at| OffsetDateTime::parse(expires_at, &Rfc3339).ok())
            .is_some_and(|expires_at| expires_at <= OffsetDateTime::now_utc())
    }

    fn validate(&self) -> Result<()> {
        reject_secret_like(&self.session_id, "working memory session id")?;
        reject_secret_like(&self.scope, "working memory scope")?;
        reject_secret_like(&self.content, "working memory content")?;
        for tag in &self.tags {
            reject_secret_like(tag, "working memory tag")?;
        }
        if let Some(source_ref) = &self.source_ref {
            source_ref.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct WorkingMemoryQuery {
    pub session_id: Option<String>,
    pub kind: Option<MemoryKind>,
    pub scope: Option<String>,
    pub include_expired: bool,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct JsonlWorkingMemoryStore {
    path: PathBuf,
}

impl JsonlWorkingMemoryStore {
    pub fn new(memory_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: memory_dir
                .into()
                .join("working")
                .join("working_memory.jsonl"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn append(&self, record: WorkingMemoryRecord) -> Result<WorkingMemoryRecord> {
        record.validate()?;
        self.ensure_parent()?;
        let encoded = serde_json::to_string(&record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        Ok(record)
    }

    pub fn list(&self, query: WorkingMemoryQuery) -> Result<Vec<WorkingMemoryRecord>> {
        let mut records = self.read_all()?;
        records.retain(|record| {
            (query.include_expired || !record.is_expired())
                && query
                    .session_id
                    .as_ref()
                    .is_none_or(|session_id| &record.session_id == session_id)
                && query.kind.as_ref().is_none_or(|kind| &record.kind == kind)
                && query
                    .scope
                    .as_ref()
                    .is_none_or(|scope| &record.scope == scope)
        });
        if let Some(limit) = query.limit {
            records.truncate(limit);
        }
        Ok(records)
    }

    pub fn prune_expired(&self) -> Result<Vec<WorkingMemoryRecord>> {
        let records = self.read_all()?;
        let (expired, active): (Vec<_>, Vec<_>) = records
            .into_iter()
            .partition(WorkingMemoryRecord::is_expired);
        if !expired.is_empty() {
            self.write_all(&active)?;
        }
        Ok(expired)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn ensure_parent(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<WorkingMemoryRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file =
            fs::File::open(&self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        let reader = BufReader::new(file);
        let mut records = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|source| IkarosError::io(&self.path, source))?;
            if line.trim().is_empty() {
                continue;
            }
            records.push(serde_json::from_str(&line)?);
        }
        Ok(records)
    }

    fn write_all(&self, records: &[WorkingMemoryRecord]) -> Result<()> {
        self.ensure_parent()?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        for record in records {
            record.validate()?;
            let encoded = serde_json::to_string(record)?;
            writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        }
        Ok(())
    }
}
