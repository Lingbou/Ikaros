// SPDX-License-Identifier: GPL-3.0-only

use crate::{MemoryKind, MemoryRef};
use ikaros_core::{IkarosError, Result, now_rfc3339, reject_secret_like};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct MemoryScore {
    pub recency: f32,
    pub relevance: f32,
    pub frequency: f32,
    pub source_strength: f32,
    #[serde(default = "default_score_component")]
    pub confidence: f32,
    #[serde(default)]
    pub sensitivity: f32,
}

impl MemoryScore {
    pub fn combined(self) -> f32 {
        ((self.recency * 0.20)
            + (self.relevance * 0.30)
            + (self.frequency * 0.15)
            + (self.source_strength * 0.15)
            + (self.confidence * 0.15)
            + ((1.0 - self.sensitivity).clamp(0.0, 1.0) * 0.05))
            .clamp(0.0, 1.0)
    }
}

impl Default for MemoryScore {
    fn default() -> Self {
        Self {
            recency: 0.5,
            relevance: 0.5,
            frequency: 0.0,
            source_strength: 0.5,
            confidence: 0.5,
            sensitivity: 0.0,
        }
    }
}

fn default_score_component() -> f32 {
    0.5
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryPolicy {
    pub promote_threshold: f32,
    pub demote_threshold: f32,
    pub forget_threshold: f32,
    pub max_records_per_scope: usize,
}

impl Default for MemoryPolicy {
    fn default() -> Self {
        Self {
            promote_threshold: 0.75,
            demote_threshold: 0.35,
            forget_threshold: 0.15,
            max_records_per_scope: 2_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryJournalAction {
    Append,
    Update,
    Promote,
    Demote,
    Forget,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryJournalEntry {
    pub id: String,
    pub at: String,
    pub action: MemoryJournalAction,
    pub memory_id: Option<String>,
    pub kind: Option<MemoryKind>,
    pub scope: Option<String>,
    pub score: Option<MemoryScore>,
    pub policy: MemoryPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<MemoryRef>,
    pub reason: String,
}

impl MemoryJournalEntry {
    pub fn new(action: MemoryJournalAction, reason: impl Into<String>) -> Result<Self> {
        let reason = reason.into();
        reject_secret_like(&reason, "memory journal reason")?;
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            at: now_rfc3339()?,
            action,
            memory_id: None,
            kind: None,
            scope: None,
            score: None,
            policy: MemoryPolicy::default(),
            source_ref: None,
            reason,
        })
    }

    pub fn with_memory(
        mut self,
        memory_id: impl Into<String>,
        kind: MemoryKind,
        scope: impl Into<String>,
    ) -> Result<Self> {
        let memory_id = memory_id.into();
        let scope = scope.into();
        reject_secret_like(&memory_id, "memory journal memory id")?;
        reject_secret_like(&scope, "memory journal scope")?;
        self.memory_id = Some(memory_id);
        self.kind = Some(kind);
        self.scope = Some(scope);
        Ok(self)
    }

    pub fn with_score(mut self, score: MemoryScore) -> Self {
        self.score = Some(score);
        self
    }

    pub fn with_source_ref(mut self, source_ref: MemoryRef) -> Result<Self> {
        source_ref.validate()?;
        self.source_ref = Some(source_ref);
        Ok(self)
    }
}

pub trait MemoryJournal {
    fn append(&self, entry: MemoryJournalEntry) -> Result<MemoryJournalEntry>;
    fn list(&self) -> Result<Vec<MemoryJournalEntry>>;
    fn path(&self) -> &Path;
}

#[derive(Debug, Clone)]
pub struct JsonlMemoryJournal {
    path: PathBuf,
}

impl JsonlMemoryJournal {
    pub fn new(memory_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: memory_dir.into().join("memory_journal.jsonl"),
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
}

impl MemoryJournal for JsonlMemoryJournal {
    fn append(&self, entry: MemoryJournalEntry) -> Result<MemoryJournalEntry> {
        reject_secret_like(&entry.reason, "memory journal reason")?;
        if let Some(memory_id) = &entry.memory_id {
            reject_secret_like(memory_id, "memory journal memory id")?;
        }
        if let Some(scope) = &entry.scope {
            reject_secret_like(scope, "memory journal scope")?;
        }
        if let Some(source_ref) = &entry.source_ref {
            source_ref.validate()?;
        }
        self.ensure_parent()?;
        let encoded = serde_json::to_string(&entry)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        Ok(entry)
    }

    fn list(&self) -> Result<Vec<MemoryJournalEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file =
            fs::File::open(&self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|source| IkarosError::io(&self.path, source))?;
            if line.trim().is_empty() {
                continue;
            }
            entries.push(serde_json::from_str(&line)?);
        }
        Ok(entries)
    }

    fn path(&self) -> &Path {
        &self.path
    }
}
