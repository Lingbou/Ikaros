// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

use super::{SandboxBackend, SandboxReadScope};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExecutionConfig {
    pub network: ExecutionNetworkConfig,
    pub sandbox: ExecutionSandboxConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExecutionNetworkConfig {
    pub enabled: bool,
    pub allow_provider_hosts: bool,
    pub allowed_hosts: Vec<String>,
    pub timeout_ms: u64,
}

impl Default for ExecutionNetworkConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            allow_provider_hosts: true,
            allowed_hosts: Vec::new(),
            timeout_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExecutionSandboxConfig {
    pub backend: SandboxBackend,
    pub image: String,
    pub read_scope: SandboxReadScope,
}

impl Default for ExecutionSandboxConfig {
    fn default() -> Self {
        Self {
            backend: SandboxBackend::Local,
            image: "rust:1.85-bookworm".into(),
            read_scope: SandboxReadScope::Workspace,
        }
    }
}
