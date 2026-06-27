// SPDX-License-Identifier: GPL-3.0-only
//! Stable MCP and JSON-RPC wire shapes shared by skills and surfaces.

use ikaros_core::{Result, redact_json, redact_secrets};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const JSONRPC_VERSION: &str = "2.0";
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcRequest {
    #[serde(default = "jsonrpc_version")]
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum JsonRpcMessage {
    Request(JsonRpcRequest),
    Other(Value),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcResponse {
    pub jsonrpc: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    pub annotations: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpPrompt {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub arguments: Vec<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpStdioProbeReport {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_info: Option<Value>,
    #[serde(default)]
    pub tools: Vec<McpTool>,
    #[serde(default)]
    pub resources: Vec<McpResource>,
    #[serde(default)]
    pub prompts: Vec<McpPrompt>,
    #[serde(default)]
    pub errors: Vec<McpStdioProbeError>,
    pub response_count: usize,
}

impl McpStdioProbeReport {
    pub fn ok(&self) -> bool {
        self.server_info.is_some() && !self.tools.is_empty() && self.errors.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpStdioProbeError {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    pub code: i64,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(
        id: Option<Value>,
        code: i64,
        message: impl Into<String>,
        data: Option<Value>,
    ) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION,
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data,
            }),
        }
    }
}

pub fn mcp_stdio_probe_input() -> Result<String> {
    let requests = [
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "ikaros",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": 2,
            "method": "tools/list",
            "params": {}
        }),
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": 3,
            "method": "resources/list",
            "params": {}
        }),
        json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": 4,
            "method": "prompts/list",
            "params": {}
        }),
    ];
    let mut input = String::new();
    for request in requests {
        input.push_str(&serde_json::to_string(&request)?);
        input.push('\n');
    }
    Ok(input)
}

pub fn parse_mcp_stdio_probe_output(stdout: &str) -> McpStdioProbeReport {
    let mut report = McpStdioProbeReport {
        protocol_version: None,
        server_info: None,
        tools: Vec::new(),
        resources: Vec::new(),
        prompts: Vec::new(),
        errors: Vec::new(),
        response_count: 0,
    };
    for raw_line in stdout.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        report.response_count += 1;
        let value = match serde_json::from_str::<Value>(line) {
            Ok(value) => value,
            Err(error) => {
                report.errors.push(McpStdioProbeError {
                    id: None,
                    code: -32700,
                    message: redact_secrets(&format!("invalid JSON-RPC response: {error}")),
                });
                continue;
            }
        };
        let id = value.get("id").cloned();
        if let Some(error) = value.get("error") {
            let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32000);
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .map(redact_secrets)
                .unwrap_or_else(|| "MCP server returned an error".into());
            report.errors.push(McpStdioProbeError { id, code, message });
            continue;
        }
        let Some(result) = value.get("result") else {
            report.errors.push(McpStdioProbeError {
                id,
                code: -32603,
                message: "MCP response is missing result/error".into(),
            });
            continue;
        };
        match id.as_ref().and_then(Value::as_i64) {
            Some(1) => {
                report.protocol_version = result
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned);
                report.server_info = result.get("serverInfo").cloned().map(redact_json);
            }
            Some(2) => {
                let tools = result.get("tools").cloned().unwrap_or_else(|| json!([]));
                match serde_json::from_value::<Vec<McpTool>>(redact_json(tools)) {
                    Ok(tools) => report.tools = tools,
                    Err(error) => report.errors.push(McpStdioProbeError {
                        id,
                        code: -32603,
                        message: redact_secrets(&format!("invalid tools/list payload: {error}")),
                    }),
                }
            }
            Some(3) => {
                let resources = result
                    .get("resources")
                    .cloned()
                    .unwrap_or_else(|| json!([]));
                match serde_json::from_value::<Vec<McpResource>>(redact_json(resources)) {
                    Ok(resources) => report.resources = resources,
                    Err(error) => report.errors.push(McpStdioProbeError {
                        id,
                        code: -32603,
                        message: redact_secrets(&format!(
                            "invalid resources/list payload: {error}"
                        )),
                    }),
                }
            }
            Some(4) => {
                let prompts = result.get("prompts").cloned().unwrap_or_else(|| json!([]));
                match serde_json::from_value::<Vec<McpPrompt>>(redact_json(prompts)) {
                    Ok(prompts) => report.prompts = prompts,
                    Err(error) => report.errors.push(McpStdioProbeError {
                        id,
                        code: -32603,
                        message: redact_secrets(&format!("invalid prompts/list payload: {error}")),
                    }),
                }
            }
            _ => {}
        }
    }
    report
}

fn jsonrpc_version() -> String {
    JSONRPC_VERSION.into()
}
