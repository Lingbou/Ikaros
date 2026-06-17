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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCandidateReason {
    ExplicitRemember,
    PreferencePattern,
    TaskOutcome,
    RuntimeInference,
    Manual,
    RagReference,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCandidateStatus {
    Pending,
    Accepted,
    Rejected,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCandidate {
    pub id: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reviewed_at: Option<String>,
    pub kind: MemoryKind,
    pub scope: String,
    pub content: String,
    pub reason: MemoryCandidateReason,
    pub confidence: f32,
    pub status: MemoryCandidateStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<MemoryRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_reason: Option<String>,
}

impl MemoryCandidate {
    pub fn new(
        kind: MemoryKind,
        scope: impl Into<String>,
        content: impl Into<String>,
        reason: MemoryCandidateReason,
        confidence: f32,
    ) -> Result<Self> {
        let scope = scope.into();
        let content = content.into();
        reject_secret_like(&scope, "memory candidate scope")?;
        reject_secret_like(&content, "memory candidate content")?;
        if !(0.0..=1.0).contains(&confidence) {
            return Err(IkarosError::Message(
                "memory candidate confidence must be between 0.0 and 1.0".into(),
            ));
        }
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            created_at: now_rfc3339()?,
            reviewed_at: None,
            kind,
            scope,
            content,
            reason,
            confidence,
            status: MemoryCandidateStatus::Pending,
            tags: Vec::new(),
            source_ref: None,
            review_reason: None,
        })
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Result<Self> {
        for tag in &tags {
            reject_secret_like(tag, "memory candidate tag")?;
        }
        self.tags = tags;
        Ok(self)
    }

    pub fn with_source_ref(mut self, source_ref: MemoryRef) -> Result<Self> {
        source_ref.validate()?;
        self.source_ref = Some(source_ref);
        Ok(self)
    }

    fn validate(&self) -> Result<()> {
        reject_secret_like(&self.scope, "memory candidate scope")?;
        reject_secret_like(&self.content, "memory candidate content")?;
        if let Some(review_reason) = &self.review_reason {
            reject_secret_like(review_reason, "memory candidate review reason")?;
        }
        for tag in &self.tags {
            reject_secret_like(tag, "memory candidate tag")?;
        }
        if let Some(source_ref) = &self.source_ref {
            source_ref.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MemoryCandidateQuery {
    pub status: Option<MemoryCandidateStatus>,
    pub kind: Option<MemoryKind>,
    pub scope: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct JsonlMemoryCandidateStore {
    path: PathBuf,
}

impl JsonlMemoryCandidateStore {
    pub fn new(memory_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: memory_dir.into().join("candidates.jsonl"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn create(&self, candidate: MemoryCandidate) -> Result<MemoryCandidate> {
        candidate.validate()?;
        self.ensure_parent()?;
        let encoded = serde_json::to_string(&candidate)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        Ok(candidate)
    }

    pub fn list(&self, query: MemoryCandidateQuery) -> Result<Vec<MemoryCandidate>> {
        let mut candidates = self.read_all()?;
        candidates.retain(|candidate| {
            query
                .status
                .as_ref()
                .is_none_or(|status| &candidate.status == status)
                && query
                    .kind
                    .as_ref()
                    .is_none_or(|kind| &candidate.kind == kind)
                && query
                    .scope
                    .as_ref()
                    .is_none_or(|scope| &candidate.scope == scope)
        });
        if let Some(limit) = query.limit {
            candidates.truncate(limit);
        }
        Ok(candidates)
    }

    pub fn set_status(
        &self,
        id: &str,
        status: MemoryCandidateStatus,
        review_reason: impl Into<String>,
    ) -> Result<Option<MemoryCandidate>> {
        reject_secret_like(id, "memory candidate id")?;
        let review_reason = review_reason.into();
        reject_secret_like(&review_reason, "memory candidate review reason")?;
        let mut candidates = self.read_all()?;
        let now = now_rfc3339()?;
        let mut updated = None;
        for candidate in &mut candidates {
            if candidate.id == id {
                candidate.status = status;
                candidate.reviewed_at = Some(now);
                candidate.review_reason = Some(review_reason);
                candidate.validate()?;
                updated = Some(candidate.clone());
                break;
            }
        }
        if updated.is_some() {
            self.write_all(&candidates)?;
        }
        Ok(updated)
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

    fn read_all(&self) -> Result<Vec<MemoryCandidate>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file =
            fs::File::open(&self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        let reader = BufReader::new(file);
        let mut candidates = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|source| IkarosError::io(&self.path, source))?;
            if line.trim().is_empty() {
                continue;
            }
            candidates.push(serde_json::from_str(&line)?);
        }
        Ok(candidates)
    }

    fn write_all(&self, candidates: &[MemoryCandidate]) -> Result<()> {
        self.ensure_parent()?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        for candidate in candidates {
            let encoded = serde_json::to_string(candidate)?;
            writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        }
        Ok(())
    }
}
