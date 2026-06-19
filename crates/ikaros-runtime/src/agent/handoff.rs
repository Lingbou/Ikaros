// SPDX-License-Identifier: GPL-3.0-only

use super::types::AgentHandoffReport;
use crate::{
    environment::{recent_policy_decisions, resolve_agent, session_and_registry_for_agent},
    task::{TaskRunOptions, execute_task_text_with_options, task_steps},
};
use ikaros_core::{IkarosConfig, IkarosPaths, Result, Task};
use ikaros_harness::{AuditEvent, CancellationToken, ExecutionOptions};
use ikaros_session::SessionSource;
use serde_json::json;
use std::path::Path;

pub async fn run_agent_handoff(
    paths: &IkarosPaths,
    workspace: &Path,
    profile: Option<&str>,
    task_text: impl Into<String>,
    dry_run: bool,
) -> Result<AgentHandoffReport> {
    run_agent_handoff_with_options(
        paths,
        workspace,
        profile,
        task_text,
        TaskRunOptions::deterministic(dry_run),
    )
    .await
}

pub async fn run_agent_handoff_with_options(
    paths: &IkarosPaths,
    workspace: &Path,
    profile: Option<&str>,
    task_text: impl Into<String>,
    options: TaskRunOptions,
) -> Result<AgentHandoffReport> {
    paths.ensure()?;
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent(&config, profile)?;
    let task_text = task_text.into();
    if options.agent_loop {
        return run_agent_loop_handoff(paths, workspace, &config, agent, task_text, options).await;
    }
    run_deterministic_handoff(paths, workspace, &config, agent, task_text, options).await
}

async fn run_agent_loop_handoff(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
    agent: ikaros_core::ResolvedAgentProfile,
    task_text: String,
    mut options: TaskRunOptions,
) -> Result<AgentHandoffReport> {
    if options.session_source.is_none() {
        options.session_source = Some(SessionSource::Subagent {
            parent_agent_id: "agent_handoff".into(),
        });
    }
    let execution = execute_task_text_with_options(
        task_text,
        options.clone(),
        paths,
        workspace,
        Some(&agent.name),
    )
    .await?;
    let (session, _) = session_and_registry_for_agent(paths, workspace, config, &agent)?;
    session.audit.append(AuditEvent::new(
        "agent_handoff",
        None,
        format!("agent handoff to {}", agent.name),
        json!({
            "agent": agent.name,
            "mode": agent.profile.mode,
            "task_id": execution.task.id,
            "dry_run": options.dry_run,
            "agent_loop": true,
            "permissions": {
                "workspace_writes": agent.profile.workspace_writes,
                "shell": agent.profile.shell,
                "network": agent.profile.network,
            },
        }),
    )?)?;
    Ok(AgentHandoffReport {
        agent: agent.name,
        mode: agent.profile.mode,
        task_id: execution.task.id,
        dry_run: options.dry_run,
        agent_loop: true,
        policy_decisions: execution.policy_decisions,
        audit_path: execution.audit_path,
        report: execution.report,
        loop_report: execution.agent_loop,
    })
}

async fn run_deterministic_handoff(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
    agent: ikaros_core::ResolvedAgentProfile,
    task_text: String,
    options: TaskRunOptions,
) -> Result<AgentHandoffReport> {
    let task = Task::new(task_text)?;
    let (session, registry) = session_and_registry_for_agent(paths, workspace, config, &agent)?;
    let session = session.with_dry_run(options.dry_run);
    session.audit.append(AuditEvent::new(
        "agent_handoff",
        None,
        format!("agent handoff to {}", agent.name),
        json!({
            "agent": agent.name,
            "mode": agent.profile.mode,
            "task_id": task.id,
            "dry_run": options.dry_run,
            "agent_loop": false,
            "permissions": {
                "workspace_writes": agent.profile.workspace_writes,
                "shell": agent.profile.shell,
                "network": agent.profile.network,
            },
        }),
    )?)?;
    let report = session
        .execute_task_steps(
            &registry,
            task.id.clone(),
            task_steps(&task.title, &task.title, &task.id),
            ExecutionOptions::default(),
            CancellationToken::new(),
        )
        .await?;
    Ok(AgentHandoffReport {
        agent: agent.name,
        mode: agent.profile.mode,
        task_id: task.id,
        dry_run: options.dry_run,
        agent_loop: false,
        policy_decisions: recent_policy_decisions(&session)?,
        audit_path: session.audit.path().to_path_buf(),
        report,
        loop_report: None,
    })
}
