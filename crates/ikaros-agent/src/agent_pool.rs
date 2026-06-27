// SPDX-License-Identifier: GPL-3.0-only

use crate::task_loop::{
    TaskAgentLoopContext, TaskExecutionContext, TaskRunOptions, execute_task_text_with_context,
};
use ikaros_core::{
    AgentMode, PolicyDecision, ResolvedAgentProfile, Result, TaskState, redact_secrets,
};
use ikaros_execution::harness::{AuditEvent, ExecutionSession, SkillRegistry, TaskExecutionReport};
use ikaros_state::session::{
    RuntimeSessionTarget, SessionEntry, SessionEntryKind, SessionId, SessionSource, SessionStore,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentHandoffReport {
    pub agent: String,
    pub mode: AgentMode,
    pub task_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub dry_run: bool,
    pub agent_loop: bool,
    pub policy_decisions: Vec<PolicyDecision>,
    pub audit_path: PathBuf,
    pub report: TaskExecutionReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_report: Option<crate::agent_loop::AgentLoopReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPoolTask {
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

impl AgentPoolTask {
    pub fn new(task: impl Into<String>, profile: Option<String>) -> Self {
        Self {
            task: task.into(),
            profile,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentPoolItemReport {
    pub index: usize,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<TaskState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<AgentHandoffReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentPoolReport {
    pub dry_run: bool,
    pub agent_loop: bool,
    pub concurrency: usize,
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub reports: Vec<AgentPoolItemReport>,
}

pub struct AgentHandoffContext<'a> {
    pub agent: &'a ResolvedAgentProfile,
    pub session: ExecutionSession,
    pub registry: SkillRegistry,
    pub agent_loop: Option<TaskAgentLoopContext<'a>>,
    pub parent_evidence_target: Option<&'a RuntimeSessionTarget>,
    pub max_delegation_depth: usize,
}

pub async fn run_agent_handoff_with_options(
    task_text: impl Into<String>,
    mut options: TaskRunOptions,
    context: AgentHandoffContext<'_>,
) -> Result<AgentHandoffReport> {
    if options.delegation_depth > context.max_delegation_depth {
        return Err(ikaros_core::IkarosError::Message(format!(
            "agent delegation depth {} exceeds configured maximum {} for agent {}",
            options.delegation_depth, context.max_delegation_depth, context.agent.name
        )));
    }
    if options.agent_loop && options.session_source.is_none() {
        options.session_source = Some(SessionSource::Subagent {
            parent_agent_id: "agent_handoff".into(),
        });
    }
    let parent_session_id = options.parent_session_id.clone();
    let session_id = options.session_id.clone();
    context.session.audit.append(AuditEvent::new(
        "agent_handoff",
        None,
        format!("agent handoff to {}", context.agent.name),
        json!({
            "agent": context.agent.name,
            "mode": context.agent.profile.mode,
            "dry_run": options.dry_run,
            "agent_loop": options.agent_loop,
            "permissions": {
                "workspace_writes": context.agent.profile.workspace_writes,
                "shell": context.agent.profile.shell,
                "network": context.agent.profile.network,
            },
        }),
    )?)?;
    let execution = execute_task_text_with_context(
        task_text,
        options.clone(),
        TaskExecutionContext {
            agent: context.agent,
            session: context.session,
            registry: context.registry,
            agent_loop: context.agent_loop,
        },
    )
    .await?;
    let report_session_id = session_id.unwrap_or_else(|| execution.task.id.clone());
    record_subagent_parent_evidence(
        context.parent_evidence_target,
        context.agent,
        parent_session_id.as_deref(),
        &report_session_id,
        &execution.task.id,
        &format!("{:?}", execution.report.state),
    )?;
    Ok(AgentHandoffReport {
        agent: context.agent.name.clone(),
        mode: context.agent.profile.mode.clone(),
        task_id: execution.task.id,
        session_id: report_session_id,
        parent_session_id,
        dry_run: options.dry_run,
        agent_loop: options.agent_loop,
        policy_decisions: execution.policy_decisions,
        audit_path: execution.audit_path,
        report: execution.report,
        loop_report: execution.agent_loop,
    })
}

pub fn pool_item_from_result(
    index: usize,
    task_text: String,
    requested_profile: Option<String>,
    result: Result<AgentHandoffReport>,
) -> AgentPoolItemReport {
    match result {
        Ok(report) => AgentPoolItemReport {
            index,
            task: redact_secrets(&task_text),
            profile: Some(report.agent.clone()),
            ok: true,
            state: Some(report.report.state.clone()),
            report: Some(report),
            error: None,
        },
        Err(error) => AgentPoolItemReport {
            index,
            task: redact_secrets(&task_text),
            profile: requested_profile,
            ok: false,
            state: None,
            report: None,
            error: Some(redact_secrets(&error.to_string())),
        },
    }
}

pub fn agent_pool_report_from_items(
    options: &TaskRunOptions,
    concurrency: usize,
    reports: Vec<AgentPoolItemReport>,
) -> AgentPoolReport {
    let succeeded = reports.iter().filter(|report| report.ok).count();
    let total = reports.len();
    AgentPoolReport {
        dry_run: options.dry_run,
        agent_loop: options.agent_loop,
        concurrency,
        total,
        succeeded,
        failed: total.saturating_sub(succeeded),
        reports,
    }
}

fn record_subagent_parent_evidence(
    target: Option<&RuntimeSessionTarget>,
    agent: &ResolvedAgentProfile,
    parent_session_id: Option<&str>,
    child_session_id: &str,
    task_id: &str,
    state: &str,
) -> Result<()> {
    let (Some(target), Some(parent_session_id)) = (target, parent_session_id) else {
        return Ok(());
    };
    let parent_session_id = SessionId::from(parent_session_id);
    if target.store.get_session(&parent_session_id)?.is_none() {
        return Ok(());
    }
    let mut entry = SessionEntry::new(parent_session_id, SessionEntryKind::Custom);
    entry.visible_text = Some(format!(
        "subagent {} completed task {} with state {}",
        agent.name, task_id, state
    ));
    entry.payload = json!({
        "kind": "subagent_result",
        "agent": agent.name,
        "child_session_id": child_session_id,
        "task_id": task_id,
        "state": state,
    });
    target.store.append_entry(&entry)
}
