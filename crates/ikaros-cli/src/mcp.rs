// SPDX-License-Identifier: GPL-3.0-only

mod http;
mod status;
mod stdio;

pub(crate) use http::run_mcp_http_call;

use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_core::IkarosPaths;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum McpCommand {
    /// Serve enabled Ikaros tools over line-delimited MCP JSON-RPC on stdio.
    ServeStdio,
    /// Show configured external MCP servers without starting them.
    Status(McpStatus),
    /// Probe a configured external MCP server by id.
    Probe(McpProbe),
    /// Probe a stdio MCP server through the harness-managed process boundary.
    ProbeStdio(McpProbeStdio),
    /// Call a stdio MCP tool through the harness-managed process boundary.
    CallStdio(McpCallStdio),
    /// Probe a HTTP MCP endpoint through runtime NetworkEgress.
    ProbeHttp(McpProbeHttp),
    /// Call a tool on a HTTP MCP endpoint through runtime NetworkEgress.
    CallHttp(McpCallHttp),
}

#[derive(Debug, Args)]
pub(crate) struct McpStatus {
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Args)]
pub(crate) struct McpProbe {
    /// Configured MCP server id from config.yaml mcp.servers.
    id: String,
    /// Probe disabled servers too. Execution still goes through policy/approval.
    #[arg(long)]
    force: bool,
    /// Override configured probe timeout in milliseconds.
    #[arg(long)]
    timeout_ms: Option<u64>,
    /// Override configured maximum captured stdout/stderr bytes.
    #[arg(long)]
    max_output_bytes: Option<usize>,
}

#[derive(Debug, Args)]
pub(crate) struct McpProbeStdio {
    /// Program name or path for the MCP stdio server.
    command: String,
    /// Arguments passed to the MCP stdio server after `--`.
    #[arg(last = true)]
    args: Vec<String>,
    /// Only expose matching tool names from the probe report.
    #[arg(long = "include-tool")]
    include_tools: Vec<String>,
    /// Hide matching tool names from the probe report.
    #[arg(long = "exclude-tool")]
    exclude_tools: Vec<String>,
    /// Probe timeout in milliseconds.
    #[arg(long)]
    timeout_ms: Option<u64>,
    /// Maximum captured stdout/stderr bytes.
    #[arg(long)]
    max_output_bytes: Option<usize>,
}

#[derive(Debug, Args)]
pub(crate) struct McpCallStdio {
    /// Program name or path for the MCP stdio server.
    command: String,
    /// Tool name to call.
    tool: String,
    /// JSON object passed as MCP tools/call arguments.
    #[arg(long, default_value = "{}")]
    arguments_json: String,
    /// Probe timeout in milliseconds.
    #[arg(long)]
    timeout_ms: Option<u64>,
    /// Maximum captured stdout/stderr bytes.
    #[arg(long)]
    max_output_bytes: Option<usize>,
    /// Arguments passed to the MCP stdio server after `--`.
    #[arg(last = true)]
    args: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct McpProbeHttp {
    /// HTTP MCP endpoint URL.
    url: String,
    /// Only expose matching tool names from the probe report.
    #[arg(long = "include-tool")]
    include_tools: Vec<String>,
    /// Hide matching tool names from the probe report.
    #[arg(long = "exclude-tool")]
    exclude_tools: Vec<String>,
    /// Maximum retained response bytes.
    #[arg(long, default_value_t = 64 * 1024)]
    max_response_bytes: usize,
}

#[derive(Debug, Args)]
pub(crate) struct McpCallHttp {
    /// HTTP MCP endpoint URL.
    url: String,
    /// Tool name to call.
    tool: String,
    /// JSON object passed as MCP tools/call arguments.
    #[arg(long, default_value = "{}")]
    arguments_json: String,
    /// Maximum retained response bytes.
    #[arg(long, default_value_t = 64 * 1024)]
    max_response_bytes: usize,
}

pub(crate) async fn mcp_command(
    command: McpCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        McpCommand::ServeStdio => stdio::serve_stdio(paths, workspace, agent_override).await,
        McpCommand::Status(args) => status::status(args, paths),
        McpCommand::Probe(args) => {
            stdio::probe_configured(args, paths, workspace, agent_override).await
        }
        McpCommand::ProbeStdio(args) => {
            stdio::probe_stdio(args, paths, workspace, agent_override).await
        }
        McpCommand::CallStdio(args) => {
            stdio::call_stdio(args, paths, workspace, agent_override).await
        }
        McpCommand::ProbeHttp(args) => {
            http::probe_http(args, paths, workspace, agent_override).await
        }
        McpCommand::CallHttp(args) => http::call_http(args, paths, workspace, agent_override).await,
    }
}
