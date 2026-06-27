// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

use super::StoreBackend;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MemoryConfig {
    pub backend: StoreBackend,
    pub policy: MemoryPolicyConfig,
    pub external_providers: Vec<ExternalMemoryProviderConfig>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: StoreBackend::Jsonl,
            policy: MemoryPolicyConfig::default(),
            external_providers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct MemoryPolicyConfig {
    pub promote_threshold: f32,
    pub demote_threshold: f32,
    pub forget_threshold: f32,
    pub max_records_per_scope: usize,
}

impl Default for MemoryPolicyConfig {
    fn default() -> Self {
        Self {
            promote_threshold: 0.75,
            demote_threshold: 0.35,
            forget_threshold: 0.15,
            max_records_per_scope: 2_000,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExternalMemoryProviderConfig {
    pub id: String,
    pub provider: String,
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
}
