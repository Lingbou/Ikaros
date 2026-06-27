// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginMarketplace {
    #[serde(default)]
    pub plugins: Vec<PluginMarketplaceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PluginMarketplaceEntry {
    pub name: String,
    pub enabled: bool,
    pub quarantined: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quarantine_reason: Option<String>,
    pub priority: i32,
    pub source: String,
    pub path: Option<PathBuf>,
    pub repository: Option<String>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
}

impl PluginMarketplaceEntry {
    pub(super) fn local_default(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ..Self::default()
        }
    }
}

impl Default for PluginMarketplaceEntry {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            quarantined: false,
            quarantine_reason: None,
            priority: 100,
            source: "local".into(),
            path: None,
            repository: None,
            homepage: None,
            license: None,
            tags: Vec::new(),
            notes: None,
        }
    }
}
