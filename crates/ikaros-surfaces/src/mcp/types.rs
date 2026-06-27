// SPDX-License-Identifier: GPL-3.0-only

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub name: String,
    pub version: String,
}

impl Default for McpServerInfo {
    fn default() -> Self {
        Self {
            name: "ikaros".into(),
            version: env!("CARGO_PKG_VERSION").into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct McpHttpProbeRequest {
    pub url: String,
    pub include_tools: Vec<String>,
    pub exclude_tools: Vec<String>,
    pub max_response_bytes: usize,
}

#[derive(Debug, Clone)]
pub struct McpHttpCallRequest {
    pub url: String,
    pub tool: String,
    pub arguments: Value,
    pub max_response_bytes: usize,
}
