// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProviderKind {
    BuiltinLocal,
    ExternalPlugin,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProviderState {
    Active,
    Disabled,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryProviderDescriptor {
    pub id: String,
    pub kind: MemoryProviderKind,
    pub backend: String,
    pub state: MemoryProviderState,
    pub path: Option<PathBuf>,
    pub endpoint: Option<String>,
    pub api_key_configured: bool,
    pub notes: Vec<String>,
}

impl MemoryProviderDescriptor {
    pub fn active_local(backend: &str, path: PathBuf) -> Self {
        Self {
            id: format!("local-{backend}"),
            kind: MemoryProviderKind::BuiltinLocal,
            backend: backend.to_owned(),
            state: MemoryProviderState::Active,
            path: Some(path),
            endpoint: None,
            api_key_configured: false,
            notes: Vec::new(),
        }
    }
}
