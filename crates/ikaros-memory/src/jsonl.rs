// SPDX-License-Identifier: GPL-3.0-only

use crate::{MemoryKind, MemoryQuery, MemoryRecord, MemoryStore, common::filter_records};
use ikaros_core::{IkarosError, Result, contains_secret_like, now_rfc3339, reject_secret_like};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct JsonlMemoryStore {
    path: PathBuf,
}

impl JsonlMemoryStore {
    pub fn new(memory_dir: impl Into<PathBuf>) -> Self {
        let dir = memory_dir.into();
        Self {
            path: dir.join("memory.jsonl"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    fn ensure_parent(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        Ok(())
    }

    fn read_all(&self) -> Result<Vec<MemoryRecord>> {
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

    fn write_all(&self, records: &[MemoryRecord]) -> Result<()> {
        self.ensure_parent()?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        for record in records {
            let encoded = serde_json::to_string(record)?;
            writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        }
        Ok(())
    }
}

impl MemoryStore for JsonlMemoryStore {
    fn append(&self, mut record: MemoryRecord) -> Result<MemoryRecord> {
        reject_secret_like(&record.content, "memory content")?;
        record.validate_metadata()?;
        if record.sensitive || contains_secret_like(&record.content) {
            return Err(IkarosError::SecretRejected("memory content".into()));
        }
        self.ensure_parent()?;
        record.updated_at = None;
        let encoded = serde_json::to_string(&record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
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
        let mut records = self.read_all()?;
        let now = now_rfc3339()?;
        let mut updated = None;
        for record in &mut records {
            if record.id == id {
                if let Some(content) = content.clone() {
                    record.content = content;
                }
                if let Some(tags) = tags.clone() {
                    record.tags = tags;
                }
                reject_secret_like(&record.content, "memory content")?;
                record.validate_metadata()?;
                if record.sensitive || contains_secret_like(&record.content) {
                    return Err(IkarosError::SecretRejected("memory content".into()));
                }
                record.updated_at = Some(now.clone());
                updated = Some(record.clone());
                break;
            }
        }
        if updated.is_some() {
            self.write_all(&records)?;
        }
        Ok(updated)
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

        let mut records = self.read_all()?;
        let now = now_rfc3339()?;
        let Some(old_index) = records.iter().position(|record| record.id == old_id) else {
            return Ok(None);
        };

        replacement.active = true;
        replacement.updated_at = None;
        replacement.valid_from.get_or_insert_with(|| now.clone());
        if !replacement.supersedes.iter().any(|id| id == old_id) {
            replacement.supersedes.push(old_id.to_owned());
        }
        replacement.validate_metadata()?;

        records[old_index].active = false;
        records[old_index].updated_at = Some(now.clone());
        records[old_index].valid_until = Some(now);
        records[old_index].superseded_by = Some(replacement.id.clone());
        records[old_index].validate_metadata()?;

        let superseded = records[old_index].clone();
        let active = replacement.clone();
        records.push(replacement);
        self.write_all(&records)?;
        Ok(Some((superseded, active)))
    }

    fn delete_by_id(&self, id: &str) -> Result<bool> {
        let records = self.read_all()?;
        let before = records.len();
        let retained = records
            .into_iter()
            .filter(|record| record.id != id)
            .collect::<Vec<_>>();
        let deleted = retained.len() != before;
        if deleted {
            self.write_all(&retained)?;
        }
        Ok(deleted)
    }

    fn delete_scope(&self, kind: Option<MemoryKind>, scope: &str) -> Result<usize> {
        let records = self.read_all()?;
        let before = records.len();
        let retained = records
            .into_iter()
            .filter(|record| {
                record.scope != scope || kind.as_ref().is_some_and(|kind| &record.kind != kind)
            })
            .collect::<Vec<_>>();
        let deleted = before.saturating_sub(retained.len());
        if deleted > 0 {
            self.write_all(&retained)?;
        }
        Ok(deleted)
    }

    fn path(&self) -> &Path {
        &self.path
    }

    fn backend_name(&self) -> &'static str {
        "jsonl"
    }
}
