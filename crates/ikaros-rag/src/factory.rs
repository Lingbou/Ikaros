// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    EmbeddingProvider, HashEmbeddingProvider, MockEmbeddingProvider, SparseEmbeddingProvider,
};
use ikaros_core::{IkarosError, Result};

pub fn embedding_provider_uses_network(provider: &str) -> bool {
    matches!(
        provider.to_ascii_lowercase().as_str(),
        "openai-compatible" | "ollama"
    )
}

pub(crate) fn with_embedding_provider<T>(
    name: &str,
    f: impl FnOnce(&dyn EmbeddingProvider) -> Result<T>,
) -> Result<T> {
    match name.to_ascii_lowercase().as_str() {
        "hash" => f(&HashEmbeddingProvider),
        "sparse" => f(&SparseEmbeddingProvider),
        "mock" => f(&MockEmbeddingProvider),
        "openai-compatible" | "ollama" => Err(IkarosError::Message(format!(
            "remote embedding provider `{name}` requires a harness ExecutionEnv; use ikaros-skills RAG egress embedding path"
        ))),
        other => Err(IkarosError::Message(format!(
            "unsupported embedding provider: {other}"
        ))),
    }
}
