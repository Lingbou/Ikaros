// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    EmbeddingProvider, HashEmbeddingProvider, MockEmbeddingProvider,
    OpenAiCompatibleEmbeddingProvider, SparseEmbeddingProvider,
};
use ikaros_core::{IkarosError, RagConfig, RemoteProviderConfig, Result};

pub fn embedding_provider_uses_network(provider: &str) -> bool {
    provider.eq_ignore_ascii_case("openai-compatible")
}

pub(crate) fn with_embedding_provider<T>(
    name: &str,
    f: impl FnOnce(&dyn EmbeddingProvider) -> Result<T>,
) -> Result<T> {
    let config = RagConfig {
        embedding_provider: name.into(),
        ..RagConfig::default()
    };
    let provider_settings = RemoteProviderConfig::default();
    with_embedding_provider_config(&config, &provider_settings, f)
}

pub(crate) fn with_embedding_provider_config<T>(
    config: &RagConfig,
    provider_settings: &RemoteProviderConfig,
    f: impl FnOnce(&dyn EmbeddingProvider) -> Result<T>,
) -> Result<T> {
    match config.embedding_provider.to_ascii_lowercase().as_str() {
        "hash" => f(&HashEmbeddingProvider),
        "sparse" => f(&SparseEmbeddingProvider),
        "mock" => f(&MockEmbeddingProvider),
        "openai-compatible" => {
            let provider = OpenAiCompatibleEmbeddingProvider::from_config(
                config.embedding_provider.clone(),
                config,
                provider_settings,
            )?;
            f(&provider)
        }
        other => Err(IkarosError::Message(format!(
            "unsupported embedding provider: {other}"
        ))),
    }
}
