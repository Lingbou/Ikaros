// SPDX-License-Identifier: GPL-3.0-only

use super::super::types::{RuntimeTaskPlan, TaskRunOptions};
use super::reporting::{audit_agent_loop_task_report, task_execution_report_from_agent_loop};
use crate::agent_loop::{AgentLoopInput, AgentLoopOptions, AgentLoopReport, run_agent_loop};
use ikaros_core::{
    IkarosConfig, IkarosPaths, Plan, PlanStep, ResolvedAgentProfile, Result, RiskLevel,
    redact_secrets,
};
use ikaros_harness::{
    AuditEvent, ExecutionSession, GuardrailConfig, SkillRegistry, TaskExecutionReport,
};
use ikaros_models::governed_provider_from_config;
use ikaros_soul::{PersonaProfile, load_or_default};
use serde_json::json;

pub(super) fn agent_loop_task_plan(task_id: &str) -> RuntimeTaskPlan {
    RuntimeTaskPlan {
        plan: Plan {
            task_id: task_id.into(),
            steps: vec![PlanStep::new(
                "Run a bounded model-guided agent loop over harness skills.",
                RiskLevel::Network,
                Some("agent_loop".into()),
            )],
        },
        executable_steps: Vec::new(),
    }
}

pub(super) struct AgentLoopTaskInput<'a> {
    pub paths: &'a IkarosPaths,
    pub task_id: &'a str,
    pub task_text: &'a str,
    pub config: &'a IkarosConfig,
    pub agent: &'a ResolvedAgentProfile,
    pub session: &'a ExecutionSession,
    pub registry: &'a SkillRegistry,
    pub options: &'a TaskRunOptions,
}

pub(super) async fn execute_agent_loop_task(
    input: AgentLoopTaskInput<'_>,
) -> Result<(TaskExecutionReport, Option<AgentLoopReport>)> {
    let persona = load_or_default(&input.paths.persona)?;
    let provider = governed_provider_from_config(
        &input.config.model.default,
        &input.config.providers.model,
        &input.paths.audit_dir,
    )?;
    input.session.audit.append(AuditEvent::new(
        "task_execution_start",
        None,
        format!("task agent loop started: {}", input.task_id),
        json!({
            "task_id": input.task_id,
            "mode": "agent_loop",
            "dry_run": input.options.dry_run,
            "max_iterations": input.options.loop_max_iterations.max(1),
        }),
    )?)?;
    let loop_report = run_agent_loop(
        AgentLoopInput {
            session_id: Some(input.task_id.to_owned()),
            task_id: Some(input.task_id.to_owned()),
            system_prompt: render_task_agent_loop_system_prompt(
                input.agent,
                &persona,
                input.options.dry_run,
            ),
            user_input: input.task_text.to_owned(),
        },
        provider.as_ref(),
        input.session,
        input.registry,
        AgentLoopOptions {
            max_iterations: input.options.loop_max_iterations.max(1),
            max_tokens: Some(768),
            temperature: Some(0.2),
            stream: false,
            guardrails: GuardrailConfig::default(),
        },
    )
    .await?;
    let report = task_execution_report_from_agent_loop(
        input.task_id,
        &loop_report,
        input.registry,
        input.session.audit.path(),
    )?;
    audit_agent_loop_task_report(input.session, &report)?;
    Ok((report, Some(loop_report)))
}

fn render_task_agent_loop_system_prompt(
    agent: &ResolvedAgentProfile,
    persona: &PersonaProfile,
    dry_run: bool,
) -> String {
    redact_secrets(&format!(
        "{}\n\nAgent profile: {} ({})\nAgent role: {}\nProfile overlay: {}\nMemory context enabled: {}\nRAG context enabled: {}\nDry run: {}\n\nUse the available harness tools to complete the user task. Gather local memory or RAG context when useful. For any write, shell, network, or database action, rely on the harness policy result and stop when approval is required or policy denies the action. Return a concise final answer when the task is complete or cannot safely proceed.",
        persona.context_summary(),
        agent.name,
        agent.mode().as_str(),
        agent.profile.description,
        agent.profile.persona_overlay,
        agent.profile.memory_context,
        agent.profile.rag_context,
        dry_run,
    ))
}
