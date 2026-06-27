// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

use super::StoreBackend;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LocalStoreConfig {
    pub backend: StoreBackend,
}

impl Default for LocalStoreConfig {
    fn default() -> Self {
        Self {
            backend: StoreBackend::Jsonl,
        }
    }
}
