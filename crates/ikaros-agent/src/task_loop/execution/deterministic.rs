// SPDX-License-Identifier: GPL-3.0-only

use super::super::{
    planning::build_task_plan,
    report::{task_emotion_reason, task_emotion_signal},
    types::{RuntimeTaskExecution, TaskRunOptions},
};
use super::agent_loop::{AgentLoopTaskInput, agent_loop_task_plan, execute_agent_loop_task};
use crate::emotion::record_emotion_signal;
use ikaros_core::RuntimeSignal;
use ikaros_core::{IkarosError, PersonaProfile, ResolvedAgentProfile, Result, Task};
use ikaros_execution::harness::{
    CancellationToken, ExecutionOptions, ExecutionSession, SkillRegistry, recent_policy_decisions,
};
use ikaros_providers::model::ModelProvider;
use ikaros_state::session::RuntimeSessionTarget;
use serde_json::json;

pub struct TaskAgentLoopContext<'a> {
    pub persona: &'a PersonaProfile,
    pub provider: &'a dyn ModelProvider,
    pub session_target: &'a RuntimeSessionTarget,
}

pub struct TaskExecutionContext<'a> {
    pub agent: &'a ResolvedAgentProfile,
    pub session: ExecutionSession,
    pub registry: SkillRegistry,
    pub agent_loop: Option<TaskAgentLoopContext<'a>>,
}

pub async fn execute_task_text_with_context(
    task_text: impl Into<String>,
    options: TaskRunOptions,
    context: TaskExecutionContext<'_>,
) -> Result<RuntimeTaskExecution> {
    let task_text = task_text.into();
    let mut task = Task::new(task_text.clone())?;
    let task_plan = if options.agent_loop {
        agent_loop_task_plan(&task.id)
    } else {
        build_task_plan(&task_text, &task.title, &task.id)
    };
    let plan = task_plan.plan.clone();
    let session = context.session.with_dry_run(options.dry_run);
    record_emotion_signal(
        &session.audit,
        RuntimeSignal::Planning,
        "task plan prepared",
        json!({
            "task_id": &task.id,
            "dry_run": options.dry_run,
            "agent_loop": options.agent_loop,
            "loop_max_iterations": options.loop_max_iterations,
            "agent": session.sandbox.agent.as_ref().map(|agent| agent.name.as_str()),
        }),
    )?;
    let (report, agent_loop) = if options.agent_loop {
        let agent_loop = context.agent_loop.as_ref().ok_or_else(|| {
            IkarosError::Message(
                "agent-loop task execution requires an assembled TaskAgentLoopContext".into(),
            )
        })?;
        execute_agent_loop_task(AgentLoopTaskInput {
            task_id: &task.id,
            task_text: &task_text,
            agent: context.agent,
            persona: agent_loop.persona,
            provider: agent_loop.provider,
            session_target: agent_loop.session_target,
            session: &session,
            registry: &context.registry,
            options: &options,
        })
        .await?
    } else {
        (
            session
                .execute_task_steps(
                    &context.registry,
                    task.id.clone(),
                    task_plan.executable_steps,
                    ExecutionOptions::default(),
                    CancellationToken::new(),
                )
                .await?,
            None,
        )
    };
    let final_emotion = record_emotion_signal(
        &session.audit,
        task_emotion_signal(&report),
        task_emotion_reason(&report),
        json!({
            "task_id": &task.id,
            "state": format!("{:?}", report.state),
            "step_count": report.steps.len(),
            "agent_loop": agent_loop.is_some(),
        }),
    )?;
    task.state = report.state.clone();
    let approvals_path = session.approvals.log().map(|log| log.path().to_path_buf());
    let policy_decisions = recent_policy_decisions(&session)?;
    let audit_path = session.audit.path().to_path_buf();
    Ok(RuntimeTaskExecution {
        task,
        plan,
        report,
        agent_loop,
        dry_run: options.dry_run,
        agent: session
            .sandbox
            .agent
            .as_ref()
            .map(|agent| agent.name.clone()),
        final_emotion,
        policy_decisions,
        audit_path,
        approvals_path,
    })
}
