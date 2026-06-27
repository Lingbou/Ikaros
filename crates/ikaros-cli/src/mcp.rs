// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_approval_hint, print_skill_result, session_and_registry};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use ikaros_core::{IkarosConfig, IkarosPaths, McpServerConfig, redact_json, redact_secrets};
use ikaros_harness::{ExecutionSession, NetworkEgressRequest, NetworkEgressResponse};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::Path;
use tokio::io::{self, BufReader};

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
        McpCommand::ServeStdio => serve_stdio(paths, workspace, agent_override).await,
        McpCommand::Status(args) => status(args, paths),
        McpCommand::Probe(args) => probe_configured(args, paths, workspace, agent_override).await,
        McpCommand::ProbeStdio(args) => probe_stdio(args, paths, workspace, agent_override).await,
        McpCommand::CallStdio(args) => call_stdio(args, paths, workspace, agent_override).await,
        McpCommand::ProbeHttp(args) => probe_http(args, paths, workspace, agent_override).await,
        McpCommand::CallHttp(args) => call_http(args, paths, workspace, agent_override).await,
    }
}

async fn serve_stdio(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let stdin = BufReader::new(io::stdin());
    let stdout = io::stdout();
    ikaros_mcp::serve_mcp_stdio(registry, session, stdin, stdout).await?;
    Ok(())
}

fn status(args: McpStatus, paths: &IkarosPaths) -> Result<()> {
    let config = IkarosConfig::load(&paths.config)?;
    if args.json {
        println!("{}", mcp_status_json(&config));
        return Ok(());
    }
    println!("mcp_servers: {}", config.mcp.servers.len());
    for server in &config.mcp.servers {
        println!(
            "- id={} enabled={} transport={} command={} args={} include_tools={} exclude_tools={} timeout_ms={} max_output_bytes={}",
            safe(&server.id),
            server.enabled,
            safe(&server.transport),
            safe(&server.command),
            server.args.len(),
            format_tool_filter(&server.include_tools),
            format_tool_filter(&server.exclude_tools),
            server.timeout_ms,
            server.max_output_bytes,
        );
    }
    Ok(())
}

async fn probe_configured(
    args: McpProbe,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let config = IkarosConfig::load(&paths.config)?;
    let Some(server) = config
        .mcp
        .servers
        .iter()
        .find(|server| server.id == args.id)
    else {
        println!(
            "mcp_probe: skipped reason=unknown_server id={}",
            safe(&args.id)
        );
        println!("next: ikaros mcp status");
        return Ok(());
    };
    if !server.enabled && !args.force {
        println!(
            "mcp_probe: skipped reason=disabled id={} next=\"ikaros mcp probe {} --force\"",
            safe(&server.id),
            safe(&server.id)
        );
        return Ok(());
    }
    if server.transport.trim() != "stdio" {
        println!(
            "mcp_probe: skipped reason=unsupported_transport id={} transport={}",
            safe(&server.id),
            safe(&server.transport)
        );
        return Ok(());
    }
    run_probe_stdio_input(
        McpProbeInput::from_config(server, args.timeout_ms, args.max_output_bytes),
        paths,
        workspace,
        agent_override,
    )
    .await
}

async fn probe_stdio(
    args: McpProbeStdio,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    run_probe_stdio_input(
        McpProbeInput::from_direct(args),
        paths,
        workspace,
        agent_override,
    )
    .await
}

async fn call_stdio(
    args: McpCallStdio,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let arguments = parse_mcp_arguments(&args.arguments_json)?;
    let mut input = json!({
        "command": args.command,
        "args": args.args,
        "tool": args.tool,
        "arguments": arguments,
    });
    if let Some(timeout_ms) = args.timeout_ms {
        input["timeout_ms"] = json!(timeout_ms);
    }
    if let Some(max_output_bytes) = args.max_output_bytes {
        input["max_output_bytes"] = json!(max_output_bytes);
    }
    let result = session
        .execute_skill(&registry, "mcp_stdio_call", input)
        .await?;
    print_skill_result(&result)?;
    print_approval_hint(&result);
    Ok(())
}

async fn probe_http(
    args: McpProbeHttp,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let (session, _) = session_and_registry(paths, workspace, agent_override)?;
    let report = run_probe_http_input(&session, args).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn call_http(
    args: McpCallHttp,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let (session, _) = session_and_registry(paths, workspace, agent_override)?;
    let report = run_call_http_input(&session, args).await?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

async fn run_probe_stdio_input(
    probe: McpProbeInput,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let mut input = json!({
        "command": probe.command,
        "args": probe.args,
        "include_tools": probe.include_tools,
        "exclude_tools": probe.exclude_tools,
    });
    if let Some(timeout_ms) = probe.timeout_ms {
        input["timeout_ms"] = json!(timeout_ms);
    }
    if let Some(max_output_bytes) = probe.max_output_bytes {
        input["max_output_bytes"] = json!(max_output_bytes);
    }
    let result = session
        .execute_skill(&registry, "mcp_stdio_probe", input)
        .await?;
    print_skill_result(&result)?;
    print_approval_hint(&result);
    Ok(())
}

async fn run_probe_http_input(session: &ExecutionSession, probe: McpProbeHttp) -> Result<Value> {
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

async fn run_call_http_input(session: &ExecutionSession, call: McpCallHttp) -> Result<Value> {
    let arguments = parse_mcp_arguments(&call.arguments_json)?;
    let initialize_request = mcp_initialize_request();
    let initialize = send_mcp_http_json_rpc(
        session,
        &call.url,
        &initialize_request,
        call.max_response_bytes,
    )
    .await?;
    let call_request = mcp_tools_call_request(&call.tool, arguments);
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

pub(crate) async fn run_mcp_http_call(
    session: &ExecutionSession,
    url: &str,
    tool: &str,
    arguments_json: &str,
    max_response_bytes: usize,
) -> Result<Value> {
    run_call_http_input(
        session,
        McpCallHttp {
            url: url.into(),
            tool: tool.into(),
            arguments_json: arguments_json.into(),
            max_response_bytes,
        },
    )
    .await
}

fn mcp_initialize_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "ikaros",
                "version": env!("CARGO_PKG_VERSION")
            }
        }
    })
}

fn mcp_tools_call_request(tool: &str, arguments: Value) -> Value {
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

fn parse_mcp_arguments(input: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(input)
        .with_context(|| format!("invalid MCP arguments JSON: {}", safe(input)))?;
    if !value.is_object() {
        anyhow::bail!("MCP arguments JSON must be an object");
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
struct McpHttpProbeResponse {
    status: u16,
    body_bytes: usize,
    retained_bytes: usize,
    truncated: bool,
    json: Option<Value>,
    body_preview: Option<String>,
}

impl McpHttpProbeResponse {
    fn from_response(response: NetworkEgressResponse, max_response_bytes: usize) -> Self {
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

    fn into_json(self) -> Value {
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

fn filter_mcp_tools(
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

struct McpProbeInput {
    command: String,
    args: Vec<String>,
    include_tools: Vec<String>,
    exclude_tools: Vec<String>,
    timeout_ms: Option<u64>,
    max_output_bytes: Option<usize>,
}

impl McpProbeInput {
    fn from_direct(args: McpProbeStdio) -> Self {
        Self {
            command: args.command,
            args: args.args,
            include_tools: args.include_tools,
            exclude_tools: args.exclude_tools,
            timeout_ms: args.timeout_ms,
            max_output_bytes: args.max_output_bytes,
        }
    }

    fn from_config(
        server: &McpServerConfig,
        timeout_ms: Option<u64>,
        max_output_bytes: Option<usize>,
    ) -> Self {
        Self {
            command: server.command.clone(),
            args: server.args.clone(),
            include_tools: server.include_tools.clone(),
            exclude_tools: server.exclude_tools.clone(),
            timeout_ms: Some(timeout_ms.unwrap_or(server.timeout_ms)),
            max_output_bytes: Some(max_output_bytes.unwrap_or(server.max_output_bytes)),
        }
    }
}

fn mcp_status_json(config: &IkarosConfig) -> String {
    serde_json::to_string(&serde_json::json!({
        "schema": "ikaros-mcp-status-v1",
        "version": 1,
        "servers": config.mcp.servers.iter().map(|server| {
            serde_json::json!({
                "id": safe(&server.id),
                "enabled": server.enabled,
                "transport": safe(&server.transport),
                "command": safe(&server.command),
                "args_count": server.args.len(),
                "include_tools": server.include_tools.iter().map(|tool| safe(tool)).collect::<Vec<_>>(),
                "exclude_tools": server.exclude_tools.iter().map(|tool| safe(tool)).collect::<Vec<_>>(),
                "timeout_ms": server.timeout_ms,
                "max_output_bytes": server.max_output_bytes,
            })
        }).collect::<Vec<_>>(),
    }))
    .unwrap_or_else(|_| r#"{"schema":"ikaros-mcp-status-v1","version":1,"error":"serialization_failed"}"#.into())
}

fn format_tool_filter(values: &[String]) -> String {
    if values.is_empty() {
        return "all".into();
    }
    values
        .iter()
        .map(|value| safe(value))
        .collect::<Vec<_>>()
        .join(",")
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
    fn mcp_initialize_request_uses_supported_protocol_version() {
        let request = mcp_initialize_request();
        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["method"], "initialize");
        assert_eq!(request["params"]["protocolVersion"], "2024-11-05");
        assert_eq!(request["params"]["clientInfo"]["name"], "ikaros");
    }

    #[test]
    fn mcp_tools_call_request_preserves_arguments_shape() {
        let request = mcp_tools_call_request("web_search", json!({"query": "ikaros"}));
        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["method"], "tools/call");
        assert_eq!(request["params"]["name"], "web_search");
        assert_eq!(request["params"]["arguments"]["query"], "ikaros");
    }

    #[test]
    fn mcp_arguments_must_be_json_object() {
        assert!(parse_mcp_arguments(r#"{"query":"ikaros"}"#).is_ok());
        assert!(parse_mcp_arguments(r#"["not", "object"]"#).is_err());
    }

    #[test]
    fn mcp_http_tool_filter_applies_include_and_exclude() {
        let tools = vec![
            json!({"name": "read", "description": "Read"}),
            json!({"name": "write", "description": "Write"}),
            json!({"name": "secret", "description": "sk-tool-secret"}),
        ];
        let filtered =
            filter_mcp_tools(tools, &["read".into(), "secret".into()], &["secret".into()]);
        assert_eq!(
            filtered,
            vec![json!({"name": "read", "description": "Read"})]
        );
    }

    #[test]
    fn mcp_http_response_redacts_non_json_preview() {
        let response = McpHttpProbeResponse::from_response(
            NetworkEgressResponse {
                status: 500,
                headers: BTreeMap::new(),
                body: "error sk-mcp-secret".into(),
                body_bytes: None,
            },
            1024,
        )
        .into_json();
        assert_eq!(response["http_status"], 500);
        assert!(!response.to_string().contains("sk-mcp-secret"));
        assert!(
            response["body_preview"]
                .as_str()
                .expect("preview")
                .contains("[REDACTED_SECRET]")
        );
    }
}
