// SPDX-License-Identifier: GPL-3.0-only

use super::types::McpServerInfo;
use ikaros_core::{IkarosError, Result, redact_json, redact_secrets};
use ikaros_execution::harness::ExecutionSession;
use ikaros_execution::toolkit::{
    SkillDescriptor, SkillDescriptorKind, SkillRegistry, ToolVisibility,
};
use ikaros_protocol::mcp::{
    JSONRPC_VERSION, JsonRpcRequest, JsonRpcResponse, MCP_PROTOCOL_VERSION, McpTool,
};
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Clone)]
pub struct McpStdioServer {
    registry: SkillRegistry,
    session: ExecutionSession,
    info: McpServerInfo,
}

impl McpStdioServer {
    pub fn new(registry: SkillRegistry, session: ExecutionSession) -> Self {
        Self {
            registry,
            session,
            info: McpServerInfo::default(),
        }
    }

    pub fn with_info(mut self, info: McpServerInfo) -> Self {
        self.info = info;
        self
    }

    pub async fn serve<R, W>(&self, reader: R, writer: W) -> Result<()>
    where
        R: AsyncBufRead + Unpin,
        W: AsyncWrite + Unpin,
    {
        let mut reader = reader.lines();
        let mut writer = writer;
        while let Some(line) = reader
            .next_line()
            .await
            .map_err(|source| IkarosError::Message(format!("failed to read MCP stdin: {source}")))?
        {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let response = self.handle_line(line).await;
            if let Some(response) = response {
                write_response(&mut writer, &response).await?;
            }
        }
        writer
            .flush()
            .await
            .map_err(|source| IkarosError::Message(format!("failed to flush MCP stdout: {source}")))
    }

    async fn handle_line(&self, line: &str) -> Option<JsonRpcResponse> {
        let request = match serde_json::from_str::<JsonRpcRequest>(line) {
            Ok(request) => request,
            Err(error) => {
                return Some(JsonRpcResponse::error(
                    None,
                    -32700,
                    "parse error",
                    Some(json!({"detail": redact_secrets(&error.to_string())})),
                ));
            }
        };
        request.id.as_ref()?;
        Some(self.handle_request(request).await)
    }

    async fn handle_request(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        if request.jsonrpc != JSONRPC_VERSION {
            return JsonRpcResponse::error(
                request.id,
                -32600,
                "invalid JSON-RPC version",
                Some(json!({"expected": JSONRPC_VERSION})),
            );
        }
        match request.method.as_str() {
            "initialize" => JsonRpcResponse::success(request.id, self.initialize_result()),
            "ping" => JsonRpcResponse::success(request.id, json!({})),
            "tools/list" => JsonRpcResponse::success(request.id, self.tools_list_result()),
            "tools/call" => self.tools_call_response(request).await,
            "resources/list" => JsonRpcResponse::success(request.id, self.resources_list_result()),
            "resources/read" => self.resources_read_response(request),
            "prompts/list" => JsonRpcResponse::success(request.id, self.prompts_list_result()),
            "prompts/get" => self.prompts_get_response(request),
            other => JsonRpcResponse::error(
                request.id,
                -32601,
                "method not found",
                Some(json!({"method": redact_secrets(other)})),
            ),
        }
    }

    fn initialize_result(&self) -> Value {
        json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": {
                    "listChanged": false
                },
                "resources": {
                    "subscribe": false,
                    "listChanged": false
                },
                "prompts": {
                    "listChanged": false
                }
            },
            "serverInfo": {
                "name": self.info.name,
                "version": self.info.version,
                "runtime": "ikaros-execution"
            }
        })
    }

    fn tools_list_result(&self) -> Value {
        let tools = self
            .registry
            .tool_registry()
            .descriptors_for(&self.session.toolsets)
            .into_iter()
            .filter(|descriptor| descriptor.kind == SkillDescriptorKind::ExecutableTool)
            .filter_map(|descriptor| {
                let visibility = self
                    .registry
                    .visibility_for(&descriptor.name, &self.session.toolsets)?;
                matches!(
                    visibility,
                    ToolVisibility::Direct | ToolVisibility::Deferred
                )
                .then_some((descriptor, visibility))
            })
            .map(|(descriptor, visibility)| {
                if visibility == ToolVisibility::Deferred {
                    self.session.disclose_deferred_tool(descriptor.name.clone());
                }
                mcp_tool_from_descriptor(descriptor)
            })
            .collect::<Vec<_>>();
        json!({ "tools": tools })
    }

    fn resources_list_result(&self) -> Value {
        json!({
            "resources": [
                {
                    "uri": "ikaros://workspace",
                    "name": "workspace",
                    "description": "Current harness workspace root.",
                    "mimeType": "application/json"
                },
                {
                    "uri": "ikaros://tools",
                    "name": "tools",
                    "description": "Ikaros tool catalog visible to this session.",
                    "mimeType": "application/json"
                },
                {
                    "uri": "ikaros://toolsets",
                    "name": "toolsets",
                    "description": "Active toolset selection for this session.",
                    "mimeType": "application/json"
                },
                {
                    "uri": "ikaros://policy",
                    "name": "policy",
                    "description": "Sandbox and approval policy summary for this session.",
                    "mimeType": "application/json"
                }
            ]
        })
    }

    fn resources_read_response(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let Some(uri) = request.params.get("uri").and_then(Value::as_str) else {
            return JsonRpcResponse::error(
                request.id,
                -32602,
                "resources/read requires params.uri",
                None,
            );
        };
        let value = match uri {
            "ikaros://workspace" => json!({
                "workspace_root": self.session.sandbox.workspace_root.display().to_string(),
                "audit": self.session.audit.path().display().to_string(),
            }),
            "ikaros://tools" => json!({
                "tools": self
                    .registry
                    .tool_registry()
                    .descriptors_for(&self.session.toolsets)
                    .into_iter()
                    .map(|descriptor| redact_json(json!({
                        "name": descriptor.name.clone(),
                        "description": descriptor.description,
                        "risk": descriptor.risk_level,
                        "kind": descriptor.kind,
                        "toolset": descriptor.toolset,
                        "execution_mode": descriptor.execution_mode,
                        "input_schema": descriptor.input_schema,
                        "visibility": self.registry.visibility_for(&descriptor.name, &self.session.toolsets),
                    })))
                    .collect::<Vec<_>>()
            }),
            "ikaros://toolsets" => json!({
                "selection": self.session.toolsets.clone(),
            }),
            "ikaros://policy" => json!({
                "sandbox": {
                    "workspace_root": self.session.sandbox.workspace_root.display().to_string(),
                    "dry_run": self.session.sandbox.dry_run,
                    "explain": self.session.sandbox.explain,
                    "protected_paths": self.session.sandbox.protected_paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
                    "agent": self.session.sandbox.agent.clone(),
                },
                "approval": "policy_driven",
            }),
            other => {
                return JsonRpcResponse::error(
                    request.id,
                    -32602,
                    "unknown resource",
                    Some(json!({"uri": redact_secrets(other)})),
                );
            }
        };
        JsonRpcResponse::success(
            request.id,
            json!({
                "contents": [
                    {
                        "uri": uri,
                        "mimeType": "application/json",
                        "text": redact_json(value).to_string(),
                    }
                ]
            }),
        )
    }

    fn prompts_list_result(&self) -> Value {
        json!({
            "prompts": [
                {
                    "name": "tool-catalog",
                    "description": "Summarize available Ikaros tools and their risk boundaries.",
                    "arguments": []
                },
                {
                    "name": "workspace-orientation",
                    "description": "Explain the current workspace and sandbox boundary.",
                    "arguments": []
                },
                {
                    "name": "coding-safety",
                    "description": "Give model-facing instructions for safe coding changes under Ikaros.",
                    "arguments": []
                }
            ]
        })
    }

    fn prompts_get_response(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let Some(name) = request.params.get("name").and_then(Value::as_str) else {
            return JsonRpcResponse::error(
                request.id,
                -32602,
                "prompts/get requires params.name",
                None,
            );
        };
        let text = match name {
            "tool-catalog" => {
                "Use tools through Ikaros only after checking the tool catalog. Deferred tools must be disclosed before use. Every tool call is policy evaluated, audited, and redacted before it reaches durable logs."
            }
            "workspace-orientation" => {
                "The active workspace is the only allowed filesystem scope. Do not infer access outside the workspace. Ask for approval when a requested action touches network, shell, write, or other non-read boundaries."
            }
            "coding-safety" => {
                "For coding work, inspect the repository first, keep patches scoped, preserve user changes, run requested validation only when allowed, and report residual risk. Do not commit unless the user explicitly asks."
            }
            other => {
                return JsonRpcResponse::error(
                    request.id,
                    -32602,
                    "unknown prompt",
                    Some(json!({"name": redact_secrets(other)})),
                );
            }
        };
        JsonRpcResponse::success(
            request.id,
            json!({
                "description": format!("Ikaros prompt: {name}"),
                "messages": [
                    {
                        "role": "user",
                        "content": {
                            "type": "text",
                            "text": text
                        }
                    }
                ]
            }),
        )
    }

    async fn tools_call_response(&self, request: JsonRpcRequest) -> JsonRpcResponse {
        let Some(name) = request.params.get("name").and_then(Value::as_str) else {
            return JsonRpcResponse::error(
                request.id,
                -32602,
                "tools/call requires params.name",
                None,
            );
        };
        let arguments = request
            .params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if !arguments.is_object() {
            return JsonRpcResponse::error(
                request.id,
                -32602,
                "tools/call params.arguments must be an object",
                None,
            );
        }
        let visibility = self.registry.visibility_for(name, &self.session.toolsets);
        if !matches!(
            visibility,
            Some(ToolVisibility::Direct | ToolVisibility::Deferred)
        ) {
            return JsonRpcResponse::error(
                request.id,
                -32602,
                "tool is not enabled for this agent",
                Some(json!({"name": redact_secrets(name)})),
            );
        }
        if visibility == Some(ToolVisibility::Deferred)
            && !self.session.is_deferred_tool_disclosed(name)
        {
            return JsonRpcResponse::error(
                request.id,
                -32602,
                "deferred tool has not been disclosed; call tools/list first",
                Some(json!({"name": redact_secrets(name)})),
            );
        }
        match self
            .session
            .execute_skill(&self.registry, name, arguments)
            .await
        {
            Ok(result) => JsonRpcResponse::success(
                request.id,
                json!({
                    "content": [
                        {
                            "type": "text",
                            "text": redact_secrets(&result.summary),
                        },
                        {
                            "type": "text",
                            "text": redact_json(result.output).to_string(),
                        }
                    ],
                    "isError": !result.ok,
                    "ikaros": {
                        "call_id": result.call_id,
                        "ok": result.ok,
                    }
                }),
            ),
            Err(error) => JsonRpcResponse::error(
                request.id,
                -32000,
                "tool execution failed",
                Some(json!({"error": redact_secrets(&error.to_string())})),
            ),
        }
    }
}

fn mcp_tool_from_descriptor(descriptor: SkillDescriptor) -> McpTool {
    McpTool {
        name: descriptor.name,
        description: descriptor.description,
        input_schema: descriptor.input_schema,
        annotations: json!({
            "risk": descriptor.risk_level,
            "toolset": descriptor.toolset,
            "execution_mode": descriptor.execution_mode,
            "visibility": "harness_managed",
            "approval": "policy_driven",
            "provenance": descriptor.provenance,
        }),
    }
}

async fn write_response<W>(writer: &mut W, response: &JsonRpcResponse) -> Result<()>
where
    W: AsyncWrite + Unpin,
{
    let line = serde_json::to_string(response)?;
    writer
        .write_all(line.as_bytes())
        .await
        .map_err(|source| IkarosError::Message(format!("failed to write MCP stdout: {source}")))?;
    writer
        .write_all(b"\n")
        .await
        .map_err(|source| IkarosError::Message(format!("failed to write MCP stdout: {source}")))?;
    writer
        .flush()
        .await
        .map_err(|source| IkarosError::Message(format!("failed to flush MCP stdout: {source}")))
}

pub async fn serve_mcp_stdio<R, W>(
    registry: SkillRegistry,
    session: ExecutionSession,
    reader: R,
    writer: W,
) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    McpStdioServer::new(registry, session)
        .serve(reader, writer)
        .await
}
