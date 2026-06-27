// SPDX-License-Identifier: GPL-3.0-only

use super::types::{McpHttpCallRequest, McpHttpProbeRequest};
use ikaros_core::{IkarosError, Result, redact_json, redact_secrets};
use ikaros_execution::harness::{ExecutionSession, NetworkEgressRequest, NetworkEgressResponse};
use ikaros_protocol::mcp::MCP_PROTOCOL_VERSION;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub async fn probe_mcp_http(
    session: &ExecutionSession,
    probe: McpHttpProbeRequest,
) -> Result<Value> {
    let initialize_request = mcp_initialize_request();
    let initialize = send_mcp_http_json_rpc(
        session,
        &probe.url,
        &initialize_request,
        probe.max_response_bytes,
    )
    .await?;
    let tools_request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });
    let tools = send_mcp_http_json_rpc(
        session,
        &probe.url,
        &tools_request,
        probe.max_response_bytes,
    )
    .await?;
    let resources_request = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "resources/list",
        "params": {}
    });
    let resources = send_mcp_http_json_rpc(
        session,
        &probe.url,
        &resources_request,
        probe.max_response_bytes,
    )
    .await?;
    let prompts_request = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "prompts/list",
        "params": {}
    });
    let prompts = send_mcp_http_json_rpc(
        session,
        &probe.url,
        &prompts_request,
        probe.max_response_bytes,
    )
    .await?;
    let tools_json = tools.json.clone().unwrap_or_else(|| json!({}));
    let filtered_tools = filter_mcp_tools(
        tools_json
            .get("result")
            .and_then(|result| result.get("tools"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        &probe.include_tools,
        &probe.exclude_tools,
    );
    Ok(json!({
        "schema": "ikaros-mcp-http-probe-v1",
        "version": 1,
        "url": safe(&probe.url),
        "network_egress": true,
        "initialize": initialize.into_json(),
        "tools_list": tools.into_json(),
        "resources_list": resources.into_json(),
        "prompts_list": prompts.into_json(),
        "tool_count": filtered_tools.len(),
        "tools": filtered_tools,
    }))
}

pub async fn call_mcp_http(session: &ExecutionSession, call: McpHttpCallRequest) -> Result<Value> {
    let initialize_request = mcp_initialize_request();
    let initialize = send_mcp_http_json_rpc(
        session,
        &call.url,
        &initialize_request,
        call.max_response_bytes,
    )
    .await?;
    let call_request = mcp_tools_call_request(&call.tool, call.arguments);
    let response =
        send_mcp_http_json_rpc(session, &call.url, &call_request, call.max_response_bytes).await?;
    Ok(json!({
        "schema": "ikaros-mcp-http-call-v1",
        "version": 1,
        "url": safe(&call.url),
        "network_egress": true,
        "tool": safe(&call.tool),
        "initialize": initialize.into_json(),
        "request": redact_json(call_request),
        "response": response.into_json(),
    }))
}

pub async fn call_mcp_http_with_arguments_json(
    session: &ExecutionSession,
    url: &str,
    tool: &str,
    arguments_json: &str,
    max_response_bytes: usize,
) -> Result<Value> {
    call_mcp_http(
        session,
        McpHttpCallRequest {
            url: url.into(),
            tool: tool.into(),
            arguments: parse_mcp_arguments(arguments_json)?,
            max_response_bytes,
        },
    )
    .await
}

pub(super) fn mcp_initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0",
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
    })
}

pub(super) fn mcp_tools_call_request(tool: &str, arguments: Value) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": tool,
            "arguments": arguments,
        }
    })
}

pub(super) fn parse_mcp_arguments(input: &str) -> Result<Value> {
    let value = serde_json::from_str::<Value>(input).map_err(|source| {
        IkarosError::Message(format!(
            "invalid MCP arguments JSON: {}: {}",
            safe(input),
            redact_secrets(&source.to_string())
        ))
    })?;
    if !value.is_object() {
        return Err(IkarosError::Message(
            "MCP arguments JSON must be an object".into(),
        ));
    }
    Ok(value)
}

async fn send_mcp_http_json_rpc(
    session: &ExecutionSession,
    url: &str,
    request: &Value,
    max_response_bytes: usize,
) -> Result<McpHttpProbeResponse> {
    let mut headers = BTreeMap::new();
    headers.insert("content-type".into(), "application/json".into());
    headers.insert(
        "accept".into(),
        "application/json, text/event-stream".into(),
    );
    let response = session
        .env
        .send_network_request(NetworkEgressRequest {
            method: "POST".into(),
            url: url.into(),
            headers,
            body: Some(request.to_string()),
            body_bytes: None,
        })
        .await?;
    Ok(McpHttpProbeResponse::from_response(
        response,
        max_response_bytes,
    ))
}

#[derive(Debug, Clone)]
pub(super) struct McpHttpProbeResponse {
    status: u16,
    body_bytes: usize,
    retained_bytes: usize,
    truncated: bool,
    json: Option<Value>,
    body_preview: Option<String>,
}

impl McpHttpProbeResponse {
    pub(super) fn from_response(
        response: NetworkEgressResponse,
        max_response_bytes: usize,
    ) -> Self {
        let body_bytes = response
            .body_bytes
            .as_ref()
            .map(Vec::len)
            .unwrap_or_else(|| response.body.len());
        let (body, truncated) = truncate_response_body(&response.body, max_response_bytes);
        let json = serde_json::from_str::<Value>(&body).ok().map(redact_json);
        let body_preview = json.is_none().then(|| safe(&body));
        Self {
            status: response.status,
            body_bytes,
            retained_bytes: body.len(),
            truncated,
            json,
            body_preview,
        }
    }

    pub(super) fn into_json(self) -> Value {
        json!({
            "http_status": self.status,
            "body_bytes": self.body_bytes,
            "retained_bytes": self.retained_bytes,
            "truncated": self.truncated,
            "json": self.json,
            "body_preview": self.body_preview,
        })
    }
}

fn truncate_response_body(body: &str, max_response_bytes: usize) -> (String, bool) {
    let max_response_bytes = max_response_bytes.clamp(1024, 1024 * 1024);
    if body.len() <= max_response_bytes {
        return (body.to_owned(), false);
    }
    let mut end = 0;
    for (index, character) in body.char_indices() {
        let next = index + character.len_utf8();
        if next > max_response_bytes {
            break;
        }
        end = next;
    }
    (body[..end].to_owned(), true)
}

pub(super) fn filter_mcp_tools(
    tools: Vec<Value>,
    include_tools: &[String],
    exclude_tools: &[String],
) -> Vec<Value> {
    tools
        .into_iter()
        .filter(|tool| {
            let name = tool.get("name").and_then(Value::as_str).unwrap_or_default();
            (include_tools.is_empty() || include_tools.iter().any(|include| include == name))
                && !exclude_tools.iter().any(|exclude| exclude == name)
        })
        .map(redact_json)
        .collect()
}

fn safe(input: &str) -> String {
    redact_secrets(input)
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect()
}
