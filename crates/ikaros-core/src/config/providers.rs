// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExternalProvidersConfig {
    pub model: RemoteProviderConfig,
    pub embedding: RemoteProviderConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RemoteProviderConfig {
    pub api_key: String,
    pub base_url: String,
}
