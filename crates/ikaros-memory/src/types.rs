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
    pub kind: MemoryKind,
    pub scope: String,
    pub content: String,
    pub tags: Vec<String>,
    pub source: Option<String>,
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
            kind,
            scope,
            content,
            tags: Vec::new(),
            source: None,
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

    pub fn validate_metadata(&self) -> Result<()> {
        reject_secret_like(&self.scope, "memory scope")?;
        for tag in &self.tags {
            reject_secret_like(tag, "memory tag")?;
        }
        if let Some(source) = &self.source {
            reject_secret_like(source, "memory source")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryQuery {
    pub kind: Option<MemoryKind>,
    pub scope: Option<String>,
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
    fn delete_by_id(&self, id: &str) -> Result<bool>;
    fn delete_scope(&self, kind: Option<MemoryKind>, scope: &str) -> Result<usize>;
    fn path(&self) -> &Path;
    fn backend_name(&self) -> &'static str;
}
