// SPDX-License-Identifier: GPL-3.0-only

use super::{McpCallStdio, McpProbe, McpProbeStdio};
use crate::{print_approval_hint, print_skill_result, session_and_registry};
use anyhow::Result;
use ikaros_core::{IkarosPaths, McpServerConfig, redact_secrets};
use ikaros_host::configured_mcp_server;
use serde_json::{Value, json};
use std::path::Path;
use tokio::io::{self, BufReader};

pub(super) async fn serve_stdio(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let stdin = BufReader::new(io::stdin());
    let stdout = io::stdout();
    ikaros_surfaces::mcp::serve_mcp_stdio(registry, session, stdin, stdout).await?;
    Ok(())
}

pub(super) async fn probe_configured(
    args: McpProbe,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let Some(server) = configured_mcp_server(paths, &args.id)? else {
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
        McpProbeInput::from_config(&server, args.timeout_ms, args.max_output_bytes),
        paths,
        workspace,
        agent_override,
    )
    .await
}

pub(super) async fn probe_stdio(
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

pub(super) async fn call_stdio(
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

fn parse_mcp_arguments(input: &str) -> Result<Value> {
    let value: Value = serde_json::from_str(input)
        .map_err(|error| anyhow::anyhow!("invalid MCP arguments JSON: {}: {error}", safe(input)))?;
    if !value.is_object() {
        anyhow::bail!("MCP arguments JSON must be an object");
    }
    Ok(value)
}

fn safe(input: &str) -> String {
    redact_secrets(input)
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect()
}
