// SPDX-License-Identifier: GPL-3.0-only

use super::super::types::{RuntimeTaskPlan, TaskRunOptions};
use super::reporting::{audit_agent_loop_task_report, task_execution_report_from_agent_loop};
use crate::{
    AgentHarness, AgentHarnessConfig, AgentLoopOptions, AgentLoopReport, EgressModelHttpClient,
    HarnessAgentRuntime, agent_toolset_selection, session::runtime_session_target,
};
use ikaros_core::{
    AgentInstance, IkarosConfig, IkarosPaths, Plan, PlanStep, ResolvedAgentProfile, Result,
    RiskLevel, redact_secrets,
};
use ikaros_harness::{
    AuditEvent, ExecutionSession, GuardrailConfig, SkillRegistry, TaskExecutionReport, Toolset,
    ToolsetSelection,
};
use ikaros_models::{ModelRequestOptions, governed_provider_from_config_with_http_client};
use ikaros_session::{PersistingAgentTurnSink, SessionId, SessionSource, TurnId};
use ikaros_soul::{PersonaProfile, load_or_default};
use serde_json::json;
use std::sync::Arc;

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
    pub agent_instance: &'a AgentInstance,
    pub agent: &'a ResolvedAgentProfile,
    pub session: &'a ExecutionSession,
    pub registry: &'a SkillRegistry,
    pub options: &'a TaskRunOptions,
}

pub(super) async fn execute_agent_loop_task(
    input: AgentLoopTaskInput<'_>,
) -> Result<(TaskExecutionReport, Option<AgentLoopReport>)> {
    let persona = load_or_default(&input.paths.persona)?;
    let model_config = input
        .agent_instance
        .model_config(&input.config.model.default);
    let model_provider = input.agent_instance.effective_model_provider_config(
        &input.config.model.default,
        &input.config.providers.model,
    );
    let provider = governed_provider_from_config_with_http_client(
        model_config,
        &model_provider,
        &input.paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(
            input.session.env.clone(),
        ))),
    )?;
    input
        .session
        .audit
        .append(input.session.correlate_audit_event(AuditEvent::new(
            "task_execution_start",
            None,
            format!("task agent loop started: {}", input.task_id),
            json!({
                "correlation_id": input.session.correlation_id(),
                "task_id": input.task_id,
                "mode": "agent_loop",
                "dry_run": input.options.dry_run,
                "max_iterations": input.options.loop_max_iterations.max(1),
            }),
        )?))?;
    let runtime = HarnessAgentRuntime;
    let toolsets = if input.options.safe_tools {
        ToolsetSelection::new([Toolset::Core])
    } else {
        agent_toolset_selection(input.agent)?
    };
    let target = runtime_session_target(
        input.paths,
        &input.session.sandbox.workspace_root,
        Some(&input.agent.name),
    )?;
    let session_id = SessionId::from(
        input
            .options
            .session_id
            .clone()
            .unwrap_or_else(|| input.task_id.to_owned()),
    );
    let turn_id = input.options.turn_id.clone().map(TurnId::from);
    let session_source = input
        .options
        .session_source
        .clone()
        .unwrap_or(SessionSource::Runtime);
    let session_store: Arc<dyn ikaros_session::SessionStore> = Arc::new(target.store.clone());
    let event_sink = PersistingAgentTurnSink::new(session_store)
        .with_source(session_source)
        .with_agent_id(target.agent_id.clone())
        .with_workspace(target.workspace.clone());
    let event_sink = if let Some(parent_session_id) = &input.options.parent_session_id {
        event_sink.with_parent_session_id(parent_session_id.as_str())
    } else {
        event_sink
    };
    let mut harness = AgentHarness::new(
        AgentHarnessConfig {
            session_id: session_id.clone(),
            turn_id,
            task_id: Some(input.task_id.to_owned()),
            system_prompt: render_task_agent_loop_system_prompt(
                input.agent,
                &persona,
                input.options.dry_run,
            ),
            options: AgentLoopOptions {
                max_iterations: input.options.loop_max_iterations.max(1),
                request_options: ModelRequestOptions::default(),
                stream: false,
                system_prompt_messages: Vec::new(),
                guardrails: GuardrailConfig::default(),
                toolsets,
                cancellation: Default::default(),
                hooks: None,
            },
        },
        &runtime,
        provider.as_ref(),
        input.session,
        input.registry,
        &event_sink,
    )
    .with_continuation_store(&target.store);
    let loop_report = match harness.run_turn(input.task_text.to_owned()).await {
        Ok(turn) => turn.report,
        Err(error) => {
            let _ = event_sink.rollback();
            return Err(error);
        }
    };
    event_sink.commit()?;
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
