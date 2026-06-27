// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    EmbeddingProvider,
    factory::with_embedding_provider,
    jsonl::LocalRagIndex,
    sqlite::SqliteRagIndex,
    types::{
        IngestOptions, IngestReport, IngestSourceFile, RagHit, RagIndexedFile, RagQuery, RagStore,
    },
};
use ikaros_core::{IkarosError, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub enum LocalRagStore {
    Jsonl(LocalRagIndex),
    Sqlite(SqliteRagIndex),
}

impl LocalRagStore {
    pub fn new(rag_dir: impl Into<PathBuf>, backend: &str) -> Result<Self> {
        let rag_dir = rag_dir.into();
        match backend {
            "jsonl" => Ok(Self::Jsonl(LocalRagIndex::new(rag_dir))),
            "sqlite" => Ok(Self::Sqlite(SqliteRagIndex::new(rag_dir))),
            other => Err(IkarosError::Message(format!(
                "unsupported RAG backend: {other}"
            ))),
        }
    }

    pub fn ingest_path_with_embedding_provider(
        &self,
        path: &Path,
        options: IngestOptions,
        embedding_provider: &str,
    ) -> Result<IngestReport> {
        match self {
            Self::Jsonl(store) => with_embedding_provider(embedding_provider, |provider| {
                store.ingest_path_with_embedding(path, options, provider)
            }),
            Self::Sqlite(store) => with_embedding_provider(embedding_provider, |provider| {
                store.ingest_path_with_embedding(path, options, provider)
            }),
        }
    }

    pub fn ingest_sources_with_embedding(
        &self,
        sources: Vec<IngestSourceFile>,
        options: IngestOptions,
        provider: &dyn EmbeddingProvider,
    ) -> Result<IngestReport> {
        match self {
            Self::Jsonl(store) => store.ingest_sources_with_embedding(sources, options, provider),
            Self::Sqlite(store) => store.ingest_sources_with_embedding(sources, options, provider),
        }
    }

    pub fn search_with_embedding_provider(
        &self,
        query: RagQuery,
        embedding_provider: &str,
    ) -> Result<Vec<RagHit>> {
        match self {
            Self::Jsonl(store) => with_embedding_provider(embedding_provider, |provider| {
                store.search_with_embedding(query, provider)
            }),
            Self::Sqlite(store) => with_embedding_provider(embedding_provider, |provider| {
                store.search_with_embedding(query, provider)
            }),
        }
    }

    pub fn search_with_embedding(
        &self,
        query: RagQuery,
        provider: &dyn EmbeddingProvider,
    ) -> Result<Vec<RagHit>> {
        match self {
            Self::Jsonl(store) => store.search_with_embedding(query, provider),
            Self::Sqlite(store) => store.search_with_embedding(query, provider),
        }
    }
}

impl RagStore for LocalRagStore {
    fn ingest_path(&self, path: &Path, options: IngestOptions) -> Result<IngestReport> {
        match self {
            Self::Jsonl(store) => store.ingest_path(path, options),
            Self::Sqlite(store) => store.ingest_path(path, options),
        }
    }

    fn search(&self, query: RagQuery) -> Result<Vec<RagHit>> {
        match self {
            Self::Jsonl(store) => store.search(query),
            Self::Sqlite(store) => store.search(query),
        }
    }

    fn delete_scope(&self, scope: &str) -> Result<usize> {
        match self {
            Self::Jsonl(store) => store.delete_scope(scope),
            Self::Sqlite(store) => store.delete_scope(scope),
        }
    }

    fn delete_path(&self, path: &Path) -> Result<usize> {
        match self {
            Self::Jsonl(store) => store.delete_path(path),
            Self::Sqlite(store) => store.delete_path(path),
        }
    }

    fn indexed_files(&self) -> Result<Vec<RagIndexedFile>> {
        match self {
            Self::Jsonl(store) => store.indexed_files(),
            Self::Sqlite(store) => store.indexed_files(),
        }
    }

    fn stale_files(&self) -> Result<Vec<PathBuf>> {
        match self {
            Self::Jsonl(store) => store.stale_files(),
            Self::Sqlite(store) => store.stale_files(),
        }
    }

    fn path(&self) -> &Path {
        match self {
            Self::Jsonl(store) => store.path(),
            Self::Sqlite(store) => store.path(),
        }
    }

    fn backend_name(&self) -> &'static str {
        match self {
            Self::Jsonl(store) => store.backend_name(),
            Self::Sqlite(store) => store.backend_name(),
        }
    }
}
