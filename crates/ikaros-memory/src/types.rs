// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{Result, now_rfc3339, reject_secret_like};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MemoryKind {
    User,
    Project,
    Task,
    Persona,
    Relationship,
    Knowledge,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryRecord {
    pub id: String,
    pub created_at: String,
    pub updated_at: Option<String>,
    #[serde(default = "default_active_memory")]
    pub active: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supersedes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    pub kind: MemoryKind,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perspective: Option<MemoryPerspective>,
    pub content: String,
    pub tags: Vec<String>,
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<MemoryRef>,
    pub confidence: Option<f32>,
    pub sensitive: bool,
}

impl MemoryRecord {
    pub fn new(
        kind: MemoryKind,
        scope: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<Self> {
        let scope = scope.into();
        let content = content.into();
        reject_secret_like(&scope, "memory scope")?;
        reject_secret_like(&content, "memory content")?;
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            created_at: now_rfc3339()?,
            updated_at: None,
            active: true,
            supersedes: Vec::new(),
            superseded_by: None,
            valid_from: None,
            valid_until: None,
            kind,
            scope,
            perspective: None,
            content,
            tags: Vec::new(),
            source: None,
            source_ref: None,
            confidence: None,
            sensitive: false,
        })
    }

    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    pub fn with_source_ref(mut self, source_ref: MemoryRef) -> Self {
        self.source_ref = Some(source_ref);
        self
    }

    pub fn with_perspective(mut self, perspective: MemoryPerspective) -> Self {
        self.perspective = Some(perspective);
        self
    }

    pub fn validate_metadata(&self) -> Result<()> {
        reject_secret_like(&self.scope, "memory scope")?;
        if let Some(perspective) = &self.perspective {
            perspective.validate()?;
        }
        for tag in &self.tags {
            reject_secret_like(tag, "memory tag")?;
        }
        if let Some(source) = &self.source {
            reject_secret_like(source, "memory source")?;
        }
        if let Some(source_ref) = &self.source_ref {
            source_ref.validate()?;
        }
        for memory_id in &self.supersedes {
            reject_secret_like(memory_id, "memory supersedes id")?;
        }
        if let Some(memory_id) = &self.superseded_by {
            reject_secret_like(memory_id, "memory superseded-by id")?;
        }
        if let Some(valid_from) = &self.valid_from {
            reject_secret_like(valid_from, "memory valid-from")?;
        }
        if let Some(valid_until) = &self.valid_until {
            reject_secret_like(valid_until, "memory valid-until")?;
        }
        Ok(())
    }
}

fn default_active_memory() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryPerspective {
    pub observer: String,
    pub subject: String,
}

impl MemoryPerspective {
    pub fn new(observer: impl Into<String>, subject: impl Into<String>) -> Result<Self> {
        let perspective = Self {
            observer: observer.into(),
            subject: subject.into(),
        };
        perspective.validate()?;
        Ok(perspective)
    }

    pub fn validate(&self) -> Result<()> {
        reject_secret_like(&self.observer, "memory perspective observer")?;
        reject_secret_like(&self.subject, "memory perspective subject")?;
        if self.observer.trim().is_empty() || self.subject.trim().is_empty() {
            return Err(ikaros_core::IkarosError::Message(
                "memory perspective observer and subject must be non-empty".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum MemoryRef {
    SessionTurn {
        session_id: String,
        turn_id: Option<String>,
    },
    SessionEntry {
        session_id: String,
        entry_id: String,
    },
    SkillCall {
        call_id: String,
    },
    Manual {
        note: String,
    },
}

impl MemoryRef {
    pub fn validate(&self) -> Result<()> {
        match self {
            MemoryRef::SessionTurn {
                session_id,
                turn_id,
            } => {
                reject_secret_like(session_id, "memory source session id")?;
                if let Some(turn_id) = turn_id {
                    reject_secret_like(turn_id, "memory source turn id")?;
                }
            }
            MemoryRef::SessionEntry {
                session_id,
                entry_id,
            } => {
                reject_secret_like(session_id, "memory source session id")?;
                reject_secret_like(entry_id, "memory source entry id")?;
            }
            MemoryRef::SkillCall { call_id } => {
                reject_secret_like(call_id, "memory source skill call id")?;
            }
            MemoryRef::Manual { note } => {
                reject_secret_like(note, "memory source note")?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryQuery {
    pub kind: Option<MemoryKind>,
    pub scope: Option<String>,
    pub perspective: Option<MemoryPerspective>,
    pub text: Option<String>,
    pub limit: Option<usize>,
}

pub trait MemoryStore {
    fn append(&self, record: MemoryRecord) -> Result<MemoryRecord>;
    fn list(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>>;
    fn search(&self, query: MemoryQuery) -> Result<Vec<MemoryRecord>>;
    fn update(
        &self,
        id: &str,
        content: Option<String>,
        tags: Option<Vec<String>>,
    ) -> Result<Option<MemoryRecord>>;
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
