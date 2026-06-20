// SPDX-License-Identifier: GPL-3.0-only
//! Client-side/local-first RAG indexing for Ikaros.

mod embedding;
mod factory;
mod files;
mod jsonl;
mod ollama;
mod openai_compatible;
mod sqlite;
mod store;
mod types;

pub use embedding::{
    EmbeddingProvider, HashEmbeddingProvider, MockEmbeddingProvider, SparseEmbeddingProvider,
};
pub use factory::embedding_provider_uses_network;
pub use jsonl::LocalRagIndex;
pub use ollama::OllamaEmbeddingProvider;
pub use openai_compatible::OpenAiCompatibleEmbeddingProvider;
pub use sqlite::SqliteRagIndex;
pub use store::LocalRagStore;
pub use types::{
    Citation, IngestOptions, IngestReport, IngestSourceFile, RagChunk, RagDocument, RagHit,
    RagIndexedFile, RagQuery, RagStore,
};

#[cfg(test)]
mod tests;
