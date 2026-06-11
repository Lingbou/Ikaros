// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    EmbeddingProvider, HashEmbeddingProvider, MockEmbeddingProvider,
    OpenAiCompatibleEmbeddingProvider, SparseEmbeddingProvider,
};
use ikaros_core::{IkarosError, RagConfig, Result};

pub fn embedding_provider_uses_network(provider: &str) -> bool {
    matches!(
        provider.to_ascii_lowercase().as_str(),
        "openai" | "openai-compatible" | "moonshot" | "siliconflow"
    )
}

pub(crate) fn with_embedding_provider<T>(
    name: &str,
    f: impl FnOnce(&dyn EmbeddingProvider) -> Result<T>,
) -> Result<T> {
    let config = RagConfig {
        embedding_provider: name.into(),
        ..RagConfig::default()
    };
    with_embedding_provider_config(&config, f)
}

pub(crate) fn with_embedding_provider_config<T>(
    config: &RagConfig,
    f: impl FnOnce(&dyn EmbeddingProvider) -> Result<T>,
) -> Result<T> {
    match config.embedding_provider.to_ascii_lowercase().as_str() {
        "hash" => f(&HashEmbeddingProvider),
        "sparse" => f(&SparseEmbeddingProvider),
        "mock" => f(&MockEmbeddingProvider),
        "openai" | "openai-compatible" | "moonshot" | "siliconflow" => {
            let provider = OpenAiCompatibleEmbeddingProvider::from_config(
                config.embedding_provider.clone(),
                config,
            )?;
            f(&provider)
        }
        other => Err(IkarosError::Message(format!(
            "unsupported embedding provider: {other}"
        ))),
    }
}
