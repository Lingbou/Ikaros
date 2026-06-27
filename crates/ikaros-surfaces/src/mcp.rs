// SPDX-License-Identifier: GPL-3.0-only
//! Harness-managed MCP surface facade.

mod http;
mod stdio;
mod types;

pub use http::{call_mcp_http, call_mcp_http_with_arguments_json, probe_mcp_http};
pub use ikaros_protocol::mcp::{
    JSONRPC_VERSION, JsonRpcError, JsonRpcMessage, JsonRpcRequest, JsonRpcResponse,
    MCP_PROTOCOL_VERSION, McpPrompt, McpResource, McpStdioProbeError, McpStdioProbeReport, McpTool,
    mcp_stdio_probe_input, parse_mcp_stdio_probe_output,
};
pub use stdio::{McpStdioServer, serve_mcp_stdio};
pub use types::{McpHttpCallRequest, McpHttpProbeRequest, McpServerInfo};

#[cfg(test)]
mod tests;
