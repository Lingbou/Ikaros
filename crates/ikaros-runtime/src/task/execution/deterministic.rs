// SPDX-License-Identifier: GPL-3.0-only

use super::super::{
    planning::build_task_plan,
    report::{task_emotion_reason, task_emotion_signal},
    types::{RuntimeTaskExecution, TaskRunOptions},
};
use super::agent_loop::{AgentLoopTaskInput, agent_loop_task_plan, execute_agent_loop_task};
use crate::{
    emotion::record_emotion_signal,
    environment::{recent_policy_decisions, runtime_harness},
};
use ikaros_core::{IkarosPaths, Result, Task};
use ikaros_harness::{CancellationToken, ExecutionOptions};
use ikaros_soul::RuntimeSignal;
use serde_json::json;
use std::path::Path;

pub async fn execute_task_text(
    task_text: impl Into<String>,
    dry_run: bool,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<RuntimeTaskExecution> {
    execute_task_text_with_options(
        task_text,
        TaskRunOptions::deterministic(dry_run),
        paths,
        workspace,
        agent_override,
    )
    .await
}

pub async fn execute_task_text_with_options(
    task_text: impl Into<String>,
    options: TaskRunOptions,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<RuntimeTaskExecution> {
    paths.ensure()?;
    let task_text = task_text.into();
    let mut task = Task::new(task_text.clone())?;
    let task_plan = if options.agent_loop {
        agent_loop_task_plan(&task.id)
    } else {
        build_task_plan(&task_text, &task.title, &task.id)
    };
    let plan = task_plan.plan.clone();
    let harness = runtime_harness(paths, workspace, agent_override)?;
    let session = harness.session.with_dry_run(options.dry_run);
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
        execute_agent_loop_task(AgentLoopTaskInput {
            paths,
            task_id: &task.id,
            task_text: &task_text,
            config: &harness.config,
            agent_instance: &harness.agent_instance,
            agent: &harness.agent,
            session: &session,
            registry: &harness.registry,
            options: &options,
        })
        .await?
    } else {
        (
            session
                .execute_task_steps(
                    &harness.registry,
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
