// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result};
use ikaros_core::{IkarosPaths, ToolResult, redact_json};
use ikaros_execution::harness::{CancellationToken, ExecutionSession, SkillRegistry};
use ikaros_host::{
    chat_model_services_for_session, host_agent_context, runtime_harness, skill_environment,
};
use ikaros_skills::{CodingSessionConfig, builtin_registry};
use ikaros_state::session::{
    AgentEvent, AgentEventKind, AgentEventSource, ApprovalRecord as SessionApprovalRecord,
    ApprovalStatus as SessionApprovalStatus, SessionId, SessionSource, SessionStore,
    SqliteSessionStore, TurnId,
};
use serde_json::json;
use std::{path::Path, sync::Arc};

pub(super) struct CodingWorkflowCommandInput {
    pub(super) objective: String,
    pub(super) mode: String,
    pub(super) diff: Option<String>,
    pub(super) apply_patch: bool,
    pub(super) run_tests: bool,
    pub(super) model_loop: bool,
    pub(super) max_iterations: Option<usize>,
    pub(super) model_token_budget: Option<u32>,
    pub(super) test_commands: Vec<String>,
    pub(super) session_id: Option<String>,
    pub(super) turn_id: Option<String>,
    pub(super) test_analysis_json: Option<String>,
}

pub(super) async fn run_coding_workflow_command(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    command: CodingWorkflowCommandInput,
) -> Result<(ToolResult, Option<std::path::PathBuf>)> {
    let cancellation = CancellationToken::new();
    if command.model_loop {
        install_coding_cancellation_signal(cancellation.clone());
    }
    let (session, registry, session_id, turn_id) =
        coding_session_and_registry_for_workflow_with_cancellation(
            paths,
            workspace,
            agent_override,
            command.session_id,
            command.turn_id,
            command.model_loop,
            cancellation,
        )?;
    let mut input = json!({
        "objective": command.objective,
        "mode": command.mode,
        "apply_patch": command.apply_patch,
        "run_tests": command.run_tests,
        "model_loop": command.model_loop,
        "session_id": session_id.as_str(),
        "turn_id": turn_id.as_str(),
    });
    if let Some(max_iterations) = command.max_iterations {
        input["max_iterations"] = json!(max_iterations);
    }
    if let Some(model_token_budget) = command.model_token_budget {
        input["model_token_budget"] = json!(model_token_budget);
    }
    if let Some(diff) = command.diff {
        input["diff"] = json!(diff);
    }
    if !command.test_commands.is_empty() {
        input["test_commands"] = json!(command.test_commands);
    }
    if let Some(test_analysis_json) = command.test_analysis_json {
        input["test_analysis"] = serde_json::from_str(&test_analysis_json)
            .with_context(|| "failed to parse --test-analysis-json")?;
    }
    let result = session
        .execute_skill(&registry, "code_workflow", input)
        .await?;
    record_coding_workflow_approval_request(
        paths,
        workspace,
        agent_override,
        &session,
        &result,
        &session_id,
        &turn_id,
    )?;
    Ok((result, None))
}

fn record_coding_workflow_approval_request(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session: &ExecutionSession,
    result: &ToolResult,
    session_id: &SessionId,
    turn_id: &TurnId,
) -> Result<()> {
    if result
        .output
        .get("decision")
        .and_then(serde_json::Value::as_str)
        != Some("ask_user")
    {
        return Ok(());
    }
    let Some(approval_id) = result
        .output
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(());
    };
    let Some(record) = session.approvals.get(approval_id)? else {
        return Ok(());
    };
    let host = host_agent_context(paths, workspace, agent_override)?;
    let store = SqliteSessionStore::new(&host.agent_instance.state_dir);
    store.append_approval(&SessionApprovalRecord {
        approval_id: approval_id.into(),
        session_id: session_id.clone(),
        turn_id: Some(turn_id.clone()),
        at: time::OffsetDateTime::now_utc(),
        status: SessionApprovalStatus::Requested,
        request: redact_json(serde_json::to_value(&record.request)?),
        decision: None,
    })?;
    store.append_agent_event(&AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Harness,
        AgentEventKind::ApprovalRequested,
        json!({
            "approval_id": approval_id,
            "tool": &record.request.call.name,
            "risk": format!("{:?}", record.request.call.risk),
        }),
    ))?;
    Ok(())
}

pub(crate) fn coding_session_and_registry_for_workflow_with_cancellation(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: Option<String>,
    turn_id: Option<String>,
    include_model_provider: bool,
    cancellation: CancellationToken,
) -> Result<(ExecutionSession, SkillRegistry, SessionId, TurnId)> {
    let harness = runtime_harness(paths, workspace, agent_override)?;
    let config = harness.config;
    let agent = harness.agent_instance;
    let session = harness.session;
    let session_id = session_id.map(SessionId::from).unwrap_or_default();
    let turn_id = turn_id.map(TurnId::from).unwrap_or_default();
    let mut env = skill_environment(paths, &agent.workspace, &config)?;
    let coding_model_provider = if include_model_provider {
        Some(Arc::from(
            chat_model_services_for_session(paths, &config, &agent, &session)?.provider,
        ))
    } else {
        None
    };
    env.coding_session = Some(CodingSessionConfig {
        store: Arc::new(SqliteSessionStore::new(&agent.state_dir)),
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        source: SessionSource::Cli,
        agent_id: Some(agent.agent_id.clone()),
        workspace: Some(agent.workspace.clone()),
        model_provider: coding_model_provider,
        cancellation,
    });
    Ok((session, builtin_registry(env), session_id, turn_id))
}

pub(crate) fn install_coding_cancellation_signal(cancellation: CancellationToken) {
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancellation.cancel();
            eprintln!(
                "coding_cancel_requested: waiting for the running provider/tool step to stop"
            );
        }
    });
}

pub(super) async fn resolve_code_diff(
    session: &ExecutionSession,
    registry: &SkillRegistry,
    diff: Option<String>,
) -> Result<String> {
    if let Some(diff) = diff {
        return Ok(diff);
    }
    let diff_result = session
        .execute_skill(registry, "git_diff", json!({"stat": false}))
        .await?;
    if !diff_result.ok {
        anyhow::bail!(
            "failed to collect current git diff: {}",
            diff_result.summary
        );
    }
    Ok(diff_result
        .output
        .get("stdout")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string())
}
