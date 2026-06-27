// SPDX-License-Identifier: GPL-3.0-only
//! Agent Client Protocol (ACP) server over stdio JSON-RPC.
//!
//! Enables IDE clients (VS Code, Zed, etc.) to drive Ikaros sessions:
//! create prompts, observe streaming events, handle approvals, and
//! inspect session timelines through the existing runtime and harness.

use crate::resolve_agent_instance;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use ikaros_core::{IkarosConfig, IkarosPaths, redact_secrets};
use ikaros_mcp::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};
use ikaros_runtime::ChatRunOptions;
use ikaros_runtime::new_chat_session_id;
use ikaros_session::{
    AgentEvent, AgentEventKind, SessionEntry, SessionEntryKind, SessionId, SessionStore,
    SqliteSessionStore,
};
use serde_json::{Value, json};
use std::{collections::HashMap, path::Path, sync::Mutex};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

const JSONRPC_VERSION: &str = "2.0";
const ACP_PROTOCOL_VERSION: &str = "2025-01-01";

#[derive(Debug, Subcommand)]
pub(crate) enum AcpCommand {
    /// Serve ACP over stdio for IDE integration.
    Serve(AcpServe),
}

#[derive(Debug, Args)]
pub(crate) struct AcpServe {
    /// Agent profile to use for new sessions.
    #[arg(long)]
    agent: Option<String>,
    /// Workspace root for file operations.
    #[arg(long)]
    workspace: Option<String>,
}

pub(crate) async fn acp_command(
    command: AcpCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        AcpCommand::Serve(args) => serve_acp(args, paths, workspace, agent_override).await,
    }
}

struct AcpSession {
    agent: String,
    workspace: String,
}

struct AcpServerState {
    paths: IkarosPaths,
    default_workspace: std::path::PathBuf,
    default_agent: Option<String>,
    sessions: Mutex<HashMap<String, AcpSession>>,
}

async fn serve_acp(
    args: AcpServe,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let effective_workspace = args
        .workspace
        .as_deref()
        .map(Path::new)
        .unwrap_or(workspace)
        .to_path_buf();
    let effective_agent = args.agent.as_deref().or(agent_override).map(String::from);
    let state = AcpServerState {
        paths: paths.clone(),
        default_workspace: effective_workspace,
        default_agent: effective_agent,
        sessions: Mutex::new(HashMap::new()),
    };
    let stdin = BufReader::new(tokio::io::stdin());
    acp_stdio_loop(stdin, tokio::io::stdout(), &state).await
}

async fn acp_stdio_loop<R, W>(reader: R, mut writer: W, state: &AcpServerState) -> Result<()>
where
    R: AsyncBufRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut reader = reader;
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match handle_acp_request(trimmed, state).await {
            Ok(Some(json_string)) => {
                writer.write_all(json_string.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
            }
            Ok(None) => {}
            Err(error) => {
                let err = JsonRpcResponse {
                    jsonrpc: JSONRPC_VERSION,
                    id: None,
                    result: None,
                    error: Some(JsonRpcError {
                        code: -32603,
                        message: redact_secrets(&error.to_string()),
                        data: None,
                    }),
                };
                let s = serde_json::to_string(&err)?;
                writer.write_all(s.as_bytes()).await?;
                writer.write_all(b"\n").await?;
                writer.flush().await?;
            }
        }
    }
    Ok(())
}

async fn handle_acp_request(raw: &str, state: &AcpServerState) -> Result<Option<String>> {
    let request: JsonRpcRequest =
        serde_json::from_str(raw).with_context(|| "failed to parse ACP JSON-RPC request")?;
    let is_notification = request.id.is_none();
    let id = request.id.clone();
    let result = dispatch_acp_method(&request, state).await?;
    if is_notification {
        return Ok(None);
    }
    let response = JsonRpcResponse {
        jsonrpc: JSONRPC_VERSION,
        id,
        result: Some(result),
        error: None,
    };
    Ok(Some(serde_json::to_string(&response)?))
}

async fn dispatch_acp_method(request: &JsonRpcRequest, state: &AcpServerState) -> Result<Value> {
    match request.method.as_str() {
        "initialize" => Ok(acp_initialize_response()),
        "initialized" => Ok(Value::Null),
        "session/new" => acp_session_new(request, state),
        "session/prompt" => acp_session_prompt(request, state).await,
        "session/list" => acp_session_list(state),
        "session/events" => acp_session_events(request, state),
        "session/replay" => acp_session_replay(request, state),
        "tools/list" => acp_tools_list(request, state),
        "approval/list" => acp_approval_list(request, state),
        "shutdown" => Ok(json!({"status": "shutting_down"})),
        _ => Err(anyhow::anyhow!("unknown ACP method: {}", request.method)),
    }
}

fn acp_initialize_response() -> Value {
    json!({
        "protocolVersion": ACP_PROTOCOL_VERSION,
        "serverInfo": {
            "name": "ikaros",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "capabilities": {
            "sessionManagement": true,
            "streamingEvents": true,
            "toolDiscovery": true,
            "approvalHandling": true,
            "sessionReplay": true,
        },
    })
}

fn acp_session_new(request: &JsonRpcRequest, state: &AcpServerState) -> Result<Value> {
    let agent = request
        .params
        .get("agent")
        .and_then(Value::as_str)
        .or(state.default_agent.as_deref())
        .unwrap_or("default");
    let workspace_str = request
        .params
        .get("workspace")
        .and_then(Value::as_str)
        .unwrap_or_else(|| state.default_workspace.to_str().unwrap_or("."));
    let session_id = format!("acp-{}", new_chat_session_id());
    {
        let mut sessions = state.sessions.lock().expect("sessions mutex");
        sessions.insert(
            session_id.clone(),
            AcpSession {
                agent: agent.to_owned(),
                workspace: workspace_str.to_owned(),
            },
        );
    }
    Ok(json!({
        "sessionId": session_id,
        "agent": agent,
        "workspace": workspace_str,
    }))
}

async fn acp_session_prompt(request: &JsonRpcRequest, state: &AcpServerState) -> Result<Value> {
    let session_id = request
        .params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("session/prompt requires sessionId"))?
        .to_owned();
    let message = request
        .params
        .get("message")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("session/prompt requires message"))?
        .to_owned();
    let (agent, workspace) = {
        let sessions = state.sessions.lock().expect("sessions mutex");
        let s = sessions
            .get(&session_id)
            .ok_or_else(|| anyhow::anyhow!("session not found: {session_id}"))?;
        (s.agent.clone(), s.workspace.clone())
    };
    let options = ChatRunOptions {
        session_id: Some(session_id.clone()),
        stream: request
            .params
            .get("stream")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        ..ChatRunOptions::default()
    };
    let result = ikaros_runtime::run_chat_message(
        &message,
        &state.paths,
        Path::new(&workspace),
        Some(&agent),
        options,
    )
    .await?;
    Ok(json!({
        "sessionId": session_id,
        "provider": result.provider,
        "model": result.model,
        "streamed": result.streamed,
        "streamChunks": result.stream_chunks.len(),
        "memoryHits": result.memory_hits,
        "ragHits": result.rag_hits,
        "content": redact_secrets(&result.content),
    }))
}

fn acp_session_list(state: &AcpServerState) -> Result<Value> {
    let sessions = state.sessions.lock().expect("sessions mutex");
    let entries: Vec<Value> = sessions
        .iter()
        .map(|(id, s)| {
            json!({
                "sessionId": id,
                "agent": s.agent,
                "workspace": s.workspace,
            })
        })
        .collect();
    Ok(json!({ "sessions": entries }))
}

fn acp_session_events(request: &JsonRpcRequest, state: &AcpServerState) -> Result<Value> {
    let session_id = request
        .params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("session/events requires sessionId"))?;
    let limit = request
        .params
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(50) as usize;
    let config = IkarosConfig::load(&state.paths.config)?;
    let agent_instance = resolve_agent_instance(
        &config,
        state.default_agent.as_deref(),
        &state.default_workspace,
        &state.paths.home,
    )?;
    let store = SqliteSessionStore::new(&agent_instance.state_dir);
    let replay = store
        .replay_session(&SessionId::from(session_id))
        .with_context(|| "failed to replay session")?;
    match replay {
        Some(replay) => {
            let events: Vec<Value> = replay
                .agent_events
                .iter()
                .rev()
                .take(limit)
                .rev()
                .map(acp_event_json)
                .collect();
            Ok(json!({
                "sessionId": session_id,
                "events": events,
                "total": replay.agent_events.len(),
            }))
        }
        None => Ok(json!({
            "sessionId": session_id,
            "events": [],
            "total": 0,
            "error": "session not found",
        })),
    }
}

fn acp_session_replay(request: &JsonRpcRequest, state: &AcpServerState) -> Result<Value> {
    let session_id = request
        .params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("session/replay requires sessionId"))?;
    let config = IkarosConfig::load(&state.paths.config)?;
    let agent_instance = resolve_agent_instance(
        &config,
        state.default_agent.as_deref(),
        &state.default_workspace,
        &state.paths.home,
    )?;
    let store = SqliteSessionStore::new(&agent_instance.state_dir);
    let replay = store
        .replay_session(&SessionId::from(session_id))
        .with_context(|| "failed to replay session")?;
    match replay {
        Some(replay) => {
            let entries: Vec<Value> = replay.entries.iter().map(acp_entry_json).collect();
            let events: Vec<Value> = replay.agent_events.iter().map(acp_event_json).collect();
            Ok(json!({
                "sessionId": session_id,
                "entries": entries,
                "events": events,
                "approvals": replay.approvals.len(),
            }))
        }
        None => Ok(json!({
            "sessionId": session_id,
            "entries": [],
            "events": [],
            "approvals": 0,
            "error": "session not found",
        })),
    }
}

fn acp_tools_list(request: &JsonRpcRequest, state: &AcpServerState) -> Result<Value> {
    let config = IkarosConfig::load(&state.paths.config)?;
    let agent_instance = resolve_agent_instance(
        &config,
        state.default_agent.as_deref(),
        &state.default_workspace,
        &state.paths.home,
    )?;
    let (_session, registry) =
        ikaros_host::session_and_registry_for_instance(&state.paths, &config, &agent_instance)?;
    let filter = request.params.get("filter").and_then(Value::as_str);
    let tools: Vec<Value> = registry
        .descriptors()
        .iter()
        .filter(|desc| {
            filter
                .map(|f| desc.name.contains(f) || desc.description.contains(f))
                .unwrap_or(true)
        })
        .map(|desc| {
            json!({
                "name": desc.name,
                "description": desc.description,
                "riskLevel": format!("{:?}", desc.risk_level),
                "requiresApproval": !matches!(desc.risk_level, ikaros_core::RiskLevel::SafeRead),
            })
        })
        .collect();
    Ok(json!({ "tools": tools }))
}

fn acp_approval_list(request: &JsonRpcRequest, state: &AcpServerState) -> Result<Value> {
    let session_id = request.params.get("sessionId").and_then(Value::as_str);
    let config = IkarosConfig::load(&state.paths.config)?;
    let agent_instance = resolve_agent_instance(
        &config,
        state.default_agent.as_deref(),
        &state.default_workspace,
        &state.paths.home,
    )?;
    let store = SqliteSessionStore::new(&agent_instance.state_dir);
    let target = match session_id {
        Some(id) => SessionId::from(id),
        None => return Ok(json!({ "approvals": [] })),
    };
    let replay = store.replay_session(&target)?;
    match replay {
        Some(replay) => {
            let approvals: Vec<Value> = replay
                .approvals
                .iter()
                .filter(|a| matches!(a.status, ikaros_session::ApprovalStatus::Requested))
                .map(|a| {
                    json!({
                        "id": a.approval_id,
                        "status": format!("{:?}", a.status),
                        "turnId": a.turn_id,
                        "request": a.request,
                    })
                })
                .collect();
            Ok(json!({ "approvals": approvals }))
        }
        None => Ok(json!({ "approvals": [], "error": "session not found" })),
    }
}

fn acp_event_json(event: &AgentEvent) -> Value {
    let kind_str = match &event.kind {
        AgentEventKind::ModelStream(_) => "model_stream",
        AgentEventKind::ModelDiagnostic(_) => "model_diagnostic",
        AgentEventKind::SessionStart => "session_start",
        AgentEventKind::TurnStart => "turn_start",
        AgentEventKind::UserMessage => "user_message",
        AgentEventKind::ToolCallStarted => "tool_call_started",
        AgentEventKind::ToolCallOutputDelta => "tool_call_output_delta",
        AgentEventKind::ToolCallCompleted => "tool_call_completed",
        AgentEventKind::ToolCallFailed => "tool_call_failed",
        AgentEventKind::ToolCallCancelled => "tool_call_cancelled",
        AgentEventKind::ApprovalRequested => "approval_requested",
        AgentEventKind::ApprovalResolved => "approval_resolved",
        AgentEventKind::ContextDiff => "context_diff",
        AgentEventKind::ContextCompacted => "context_compacted",
        AgentEventKind::MemoryLifecycle => "memory_lifecycle",
        AgentEventKind::CodingTurn => "coding_turn",
        AgentEventKind::AuditAnchor => "audit_anchor",
        AgentEventKind::ContinuationStarted => "continuation_started",
        AgentEventKind::ContinuationCompleted => "continuation_completed",
        AgentEventKind::ContinuationFailed => "continuation_failed",
        AgentEventKind::ContinuationCancelled => "continuation_cancelled",
        AgentEventKind::TurnEnd => "turn_end",
        AgentEventKind::Error => "error",
    };
    json!({
        "eventId": event.event_id,
        "turnId": event.turn_id,
        "kind": kind_str,
        "at": event.at,
    })
}

fn acp_entry_json(entry: &SessionEntry) -> Value {
    let kind_str = match entry.kind {
        SessionEntryKind::UserMessage => "user_message",
        SessionEntryKind::AssistantMessage => "assistant_message",
        SessionEntryKind::ToolResult => "tool_result",
        SessionEntryKind::SystemMessage => "system_message",
        SessionEntryKind::ModelChange => "model_change",
        SessionEntryKind::Compaction => "compaction",
        SessionEntryKind::BranchSummary => "branch_summary",
        SessionEntryKind::Custom => "custom",
        SessionEntryKind::Leaf => "leaf",
    };
    json!({
        "entryId": entry.entry_id,
        "kind": kind_str,
        "at": entry.at,
        "summary": redact_secrets(entry.visible_text.as_deref().unwrap_or("")),
    })
}
