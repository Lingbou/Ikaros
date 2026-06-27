// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosConfig, IkarosPaths, McpServerConfig, Result, redact_secrets};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct McpServersReport {
    pub schema: &'static str,
    pub version: u32,
    pub servers: Vec<McpServerReport>,
}

#[derive(Debug, Clone, Serialize)]
pub struct McpServerReport {
    pub id: String,
    pub enabled: bool,
    pub transport: String,
    pub command: String,
    pub args_count: usize,
    pub include_tools: Vec<String>,
    pub exclude_tools: Vec<String>,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
}

pub fn mcp_servers_report(paths: &IkarosPaths) -> Result<McpServersReport> {
    let config = IkarosConfig::load(&paths.config)?;
    Ok(mcp_servers_report_from_config(&config))
}

pub fn configured_mcp_server(paths: &IkarosPaths, id: &str) -> Result<Option<McpServerConfig>> {
    let config = IkarosConfig::load(&paths.config)?;
    Ok(config
        .mcp
        .servers
        .iter()
        .find(|server| server.id == id)
        .cloned())
}

pub fn mcp_servers_report_from_config(config: &IkarosConfig) -> McpServersReport {
    McpServersReport {
        schema: "ikaros-mcp-status-v1",
        version: 1,
        servers: config.mcp.servers.iter().map(mcp_server_report).collect(),
    }
}

fn mcp_server_report(server: &McpServerConfig) -> McpServerReport {
    McpServerReport {
        id: safe(&server.id),
        enabled: server.enabled,
        transport: safe(&server.transport),
        command: safe(&server.command),
        args_count: server.args.len(),
        include_tools: server.include_tools.iter().map(|tool| safe(tool)).collect(),
        exclude_tools: server.exclude_tools.iter().map(|tool| safe(tool)).collect(),
        timeout_ms: server.timeout_ms,
        max_output_bytes: server.max_output_bytes,
    }
}

fn safe(input: &str) -> String {
    redact_secrets(input)
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_report_redacts_secret_like_server_fields() {
        let mut config = IkarosConfig::default();
        config.mcp.servers.push(McpServerConfig {
            id: "server-sk-secret".into(),
            command: "cmd-token=abc123".into(),
            args: vec!["--secret=sk-test".into()],
            include_tools: vec!["read".into()],
            exclude_tools: vec!["danger-sk-tool".into()],
            ..McpServerConfig::default()
        });

        let report = mcp_servers_report_from_config(&config);
        let rendered = serde_json::to_string(&report).expect("json");

        assert!(!rendered.contains("sk-secret"));
        assert!(!rendered.contains("abc123"));
        assert!(!rendered.contains("sk-test"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
    }
}
