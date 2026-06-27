// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct McpConfig {
    pub servers: Vec<McpServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct McpServerConfig {
    pub id: String,
    pub enabled: bool,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    pub include_tools: Vec<String>,
    pub exclude_tools: Vec<String>,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            enabled: false,
            transport: "stdio".into(),
            command: String::new(),
            args: Vec::new(),
            include_tools: Vec::new(),
            exclude_tools: Vec::new(),
            timeout_ms: 5_000,
            max_output_bytes: 64 * 1024,
        }
    }
}
