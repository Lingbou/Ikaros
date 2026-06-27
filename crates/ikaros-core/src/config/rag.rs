// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

use super::{EmbeddingProviderKind, StoreBackend};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RagConfig {
    pub backend: StoreBackend,
    pub embedding_provider: EmbeddingProviderKind,
    pub embedding_model: String,
    pub embedding_timeout_ms: u64,
    pub embedding_max_retries: u8,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            backend: StoreBackend::Jsonl,
            embedding_provider: EmbeddingProviderKind::Hash,
            embedding_model: "text-embedding-3-small".into(),
            embedding_timeout_ms: 30_000,
            embedding_max_retries: 0,
        }
    }
}
