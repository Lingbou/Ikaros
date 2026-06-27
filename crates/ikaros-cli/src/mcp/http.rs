// SPDX-License-Identifier: GPL-3.0-only

use super::{McpCallHttp, McpProbeHttp};
use crate::session_and_registry;
use anyhow::Result;
use ikaros_core::IkarosPaths;
use ikaros_execution::harness::ExecutionSession;
use ikaros_surfaces::mcp::{
    McpHttpProbeRequest, call_mcp_http_with_arguments_json, probe_mcp_http,
};
use serde_json::Value;
use std::path::Path;

pub(super) async fn probe_http(
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

pub(super) async fn call_http(
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

async fn run_probe_http_input(session: &ExecutionSession, probe: McpProbeHttp) -> Result<Value> {
    Ok(probe_mcp_http(
        session,
        McpHttpProbeRequest {
            url: probe.url,
            include_tools: probe.include_tools,
            exclude_tools: probe.exclude_tools,
            max_response_bytes: probe.max_response_bytes,
        },
    )
    .await?)
}

async fn run_call_http_input(session: &ExecutionSession, call: McpCallHttp) -> Result<Value> {
    Ok(call_mcp_http_with_arguments_json(
        session,
        &call.url,
        &call.tool,
        &call.arguments_json,
        call.max_response_bytes,
    )
    .await?)
}

pub(crate) async fn run_mcp_http_call(
    session: &ExecutionSession,
    url: &str,
    tool: &str,
    arguments_json: &str,
    max_response_bytes: usize,
) -> Result<Value> {
    Ok(
        call_mcp_http_with_arguments_json(session, url, tool, arguments_json, max_response_bytes)
            .await?,
    )
}
