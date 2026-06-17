// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    HashEmbeddingProvider,
    embedding::{
        EmbeddingProvider, combined_score, cosine_similarity, lexical_score, query_tokens,
    },
    files::{canonical_or_self, chunk_text, collect_files, system_time_to_rfc3339},
    types::{
        Citation, IngestOptions, IngestReport, IngestSourceFile, RagChunk, RagDocument, RagHit,
        RagIndexedFile, RagQuery, RagStore,
    },
};
use ikaros_core::{IkarosError, Result, now_rfc3339, redact_secrets};
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct LocalRagIndex {
    path: PathBuf,
}

impl LocalRagIndex {
    pub fn new(rag_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: rag_dir.into().join("index.jsonl"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn ingest_path(&self, path: &Path, options: IngestOptions) -> Result<IngestReport> {
        self.ingest_path_with_embedding(path, options, &HashEmbeddingProvider)
    }

    pub fn ingest_path_with_embedding(
        &self,
        path: &Path,
        options: IngestOptions,
        embedding_provider: &dyn EmbeddingProvider,
    ) -> Result<IngestReport> {
        let mut files = Vec::new();
        collect_files(path, &mut files)?;
        let mut sources = Vec::new();
        for file in files {
            let text = match fs::read_to_string(&file) {
                Ok(text) => text,
                Err(_) => continue,
            };
            let metadata = fs::metadata(&file).map_err(|source| IkarosError::io(&file, source))?;
            sources.push(IngestSourceFile {
                source_path: file,
                content: text,
                modified_at: metadata.modified().ok().and_then(system_time_to_rfc3339),
            });
        }
        self.ingest_sources_with_embedding(sources, options, embedding_provider)
    }

    pub fn ingest_sources_with_embedding(
        &self,
        sources: Vec<IngestSourceFile>,
        options: IngestOptions,
        embedding_provider: &dyn EmbeddingProvider,
    ) -> Result<IngestReport> {
        let mut chunks = self.read_all()?;
        let canonical_targets = sources
            .iter()
            .map(|source| canonical_or_self(&source.source_path))
            .collect::<BTreeSet<_>>();
        chunks.retain(|chunk| {
            chunk.scope != options.scope
                || !canonical_targets.contains(&canonical_or_self(&chunk.source_path))
        });

        let mut indexed_files = 0;
        let mut chunks_indexed = 0;
        let files_seen = sources.len();
        for source in sources {
            let document_id = Uuid::new_v4().to_string();
            let indexed_at = now_rfc3339()?;
            let document = RagDocument {
                id: document_id.clone(),
                source_path: source.source_path,
                scope: options.scope.clone(),
                indexed_at: indexed_at.clone(),
                modified_at: source.modified_at,
            };
            let mut file_chunks = Vec::new();
            for (line_start, line_end, content) in
                chunk_text(&source.content, options.max_chunk_lines)
            {
                let content = redact_secrets(&content);
                file_chunks.push(RagChunk {
                    id: Uuid::new_v4().to_string(),
                    document_id: document.id.clone(),
                    scope: document.scope.clone(),
                    source_path: document.source_path.clone(),
                    line_start,
                    line_end,
                    embedding_provider: Some(embedding_provider.name().into()),
                    embedding: Some(embedding_provider.embed(&content)?),
                    content,
                    indexed_at: document.indexed_at.clone(),
                    modified_at: document.modified_at.clone(),
                });
            }
            if !file_chunks.is_empty() {
                indexed_files += 1;
                chunks_indexed += file_chunks.len();
                chunks.extend(file_chunks);
            }
        }

        self.write_all(&chunks)?;
        Ok(IngestReport {
            files_seen,
            files_indexed: indexed_files,
            chunks_indexed,
        })
    }

    pub fn search(&self, query: RagQuery) -> Result<Vec<RagHit>> {
        self.search_with_embedding(query, &HashEmbeddingProvider)
    }

    pub fn search_with_embedding(
        &self,
        query: RagQuery,
        embedding_provider: &dyn EmbeddingProvider,
    ) -> Result<Vec<RagHit>> {
        search_chunks(self.read_all()?, query, embedding_provider)
    }

    pub fn delete_scope(&self, scope: &str) -> Result<usize> {
        let chunks = self.read_all()?;
        let before = chunks.len();
        let retained = chunks
            .into_iter()
            .filter(|chunk| chunk.scope != scope)
            .collect::<Vec<_>>();
        self.write_all(&retained)?;
        Ok(before.saturating_sub(retained.len()))
    }

    pub fn delete_path(&self, path: &Path) -> Result<usize> {
        let target = canonical_or_self(path);
        let chunks = self.read_all()?;
        let before = chunks.len();
        let retained = chunks
            .into_iter()
            .filter(|chunk| canonical_or_self(&chunk.source_path) != target)
            .collect::<Vec<_>>();
        self.write_all(&retained)?;
        Ok(before.saturating_sub(retained.len()))
    }

    pub fn indexed_files(&self) -> Result<Vec<RagIndexedFile>> {
        let mut files = self
            .read_all()?
            .into_iter()
            .map(|chunk| RagIndexedFile {
                source_path: chunk.source_path,
                modified_at: chunk.modified_at,
            })
            .collect::<Vec<_>>();
        files.sort_by(|left, right| {
            left.source_path
                .cmp(&right.source_path)
                .then_with(|| left.modified_at.cmp(&right.modified_at))
        });
        files.dedup();
        Ok(files)
    }

    pub fn stale_files(&self) -> Result<Vec<PathBuf>> {
        let mut stale = Vec::new();
        for chunk in self.read_all()? {
            let Some(indexed_modified) = &chunk.modified_at else {
                continue;
            };
            let current = fs::metadata(&chunk.source_path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(system_time_to_rfc3339);
            if current.as_ref() != Some(indexed_modified) {
                stale.push(chunk.source_path);
            }
        }
        stale.sort();
        stale.dedup();
        Ok(stale)
    }

    fn ensure_parent(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        Ok(())
    }

    pub(crate) fn read_all(&self) -> Result<Vec<RagChunk>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file =
            fs::File::open(&self.path).map_err(|source| IkarosError::io(&self.path, source))?;
        let reader = BufReader::new(file);
        let mut chunks = Vec::new();
        for line in reader.lines() {
            let line = line.map_err(|source| IkarosError::io(&self.path, source))?;
            if line.trim().is_empty() {
                continue;
            }
            chunks.push(serde_json::from_str(&line)?);
        }
        Ok(chunks)
    }

    fn write_all(&self, chunks: &[RagChunk]) -> Result<()> {
        self.ensure_parent()?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&self.path)
            .map_err(|source| IkarosError::io(&self.path, source))?;
        for chunk in chunks {
            let encoded = serde_json::to_string(chunk)?;
            writeln!(file, "{encoded}").map_err(|source| IkarosError::io(&self.path, source))?;
        }
        Ok(())
    }
}

impl RagStore for LocalRagIndex {
    fn ingest_path(&self, path: &Path, options: IngestOptions) -> Result<IngestReport> {
        LocalRagIndex::ingest_path(self, path, options)
    }

    fn search(&self, query: RagQuery) -> Result<Vec<RagHit>> {
        LocalRagIndex::search(self, query)
    }

    fn delete_scope(&self, scope: &str) -> Result<usize> {
        LocalRagIndex::delete_scope(self, scope)
    }

    fn delete_path(&self, path: &Path) -> Result<usize> {
        LocalRagIndex::delete_path(self, path)
    }

    fn indexed_files(&self) -> Result<Vec<RagIndexedFile>> {
        LocalRagIndex::indexed_files(self)
    }

    fn stale_files(&self) -> Result<Vec<PathBuf>> {
        LocalRagIndex::stale_files(self)
    }

    fn path(&self) -> &Path {
        LocalRagIndex::path(self)
    }

    fn backend_name(&self) -> &'static str {
        "jsonl"
    }
}

pub(crate) fn search_chunks(
    chunks: Vec<RagChunk>,
    query: RagQuery,
    embedding_provider: &dyn EmbeddingProvider,
) -> Result<Vec<RagHit>> {
    let top_k = if query.top_k == 0 { 5 } else { query.top_k };
    let sanitized_query = redact_secrets(&query.query);
    let query_tokens = query_tokens(&sanitized_query);
    let query_embedding = embedding_provider.embed(&sanitized_query)?;
    if query_tokens.is_empty() {
        return Ok(Vec::new());
    }
    let mut hits = chunks
        .into_iter()
        .filter(|chunk| {
            query
                .scope
                .as_ref()
                .is_none_or(|scope| &chunk.scope == scope)
        })
        .filter_map(|chunk| {
            let lexical = lexical_score(&query_tokens, &chunk.content);
            let vector = chunk.embedding.as_ref().map_or(0.0, |embedding| {
                cosine_similarity(&query_embedding, embedding)
            });
            let score = combined_score(lexical, vector);
            (score > 0.0).then(|| {
                let citation = Citation {
                    source_path: chunk.source_path.clone(),
                    line_start: chunk.line_start,
                    line_end: chunk.line_end,
                    indexed_at: chunk.indexed_at.clone(),
                    modified_at: chunk.modified_at.clone(),
                };
                RagHit {
                    chunk,
                    score,
                    citation,
                }
            })
        })
        .collect::<Vec<_>>();
    hits.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.chunk.source_path.cmp(&right.chunk.source_path))
    });
    hits.truncate(top_k);
    Ok(hits)
}
