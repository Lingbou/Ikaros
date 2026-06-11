// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RagDocument {
    pub id: String,
    pub source_path: PathBuf,
    pub scope: String,
    pub indexed_at: String,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RagChunk {
    pub id: String,
    pub document_id: String,
    pub scope: String,
    pub source_path: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub content: String,
    pub indexed_at: String,
    pub modified_at: Option<String>,
    #[serde(default)]
    pub embedding_provider: Option<String>,
    #[serde(default)]
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RagHit {
    pub chunk: RagChunk,
    pub score: f32,
    pub citation: Citation,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Citation {
    pub source_path: PathBuf,
    pub line_start: usize,
    pub line_end: usize,
    pub indexed_at: String,
    pub modified_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestOptions {
    pub scope: String,
    pub max_chunk_lines: usize,
}

impl Default for IngestOptions {
    fn default() -> Self {
        Self {
            scope: "project".into(),
            max_chunk_lines: 40,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RagQuery {
    pub query: String,
    pub top_k: usize,
    pub scope: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IngestReport {
    pub files_seen: usize,
    pub files_indexed: usize,
    pub chunks_indexed: usize,
}

pub trait RagStore {
    fn ingest_path(&self, path: &Path, options: IngestOptions) -> Result<IngestReport>;
    fn search(&self, query: RagQuery) -> Result<Vec<RagHit>>;
    fn delete_scope(&self, scope: &str) -> Result<usize>;
    fn delete_path(&self, path: &Path) -> Result<usize>;
    fn stale_files(&self) -> Result<Vec<PathBuf>>;
    fn path(&self) -> &Path;
    fn backend_name(&self) -> &'static str;
}
