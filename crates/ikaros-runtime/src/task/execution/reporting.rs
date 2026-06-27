// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::{AgentLoopReport, AgentLoopStopReason, AgentLoopToolResult};
use ikaros_core::{Result, RiskLevel, TaskState, now_rfc3339, redact_secrets};
use ikaros_harness::{
    AuditEvent, ExecutionSession, PlanStepStatus, SkillRegistry, StepExecutionRecord,
    TaskExecutionReport,
};
use serde_json::json;
use std::path::Path;
use uuid::Uuid;

pub(super) fn task_execution_report_from_agent_loop(
    task_id: &str,
    report: &AgentLoopReport,
    registry: &SkillRegistry,
    audit_path: &Path,
) -> Result<TaskExecutionReport> {
    let mut steps = report
        .tool_results
        .iter()
        .map(|result| agent_loop_tool_result_step(result, registry))
        .collect::<Result<Vec<_>>>()?;
    if steps.is_empty() {
        steps.push(agent_loop_final_step(report)?);
    }
    let mut state = task_state_from_agent_loop(report);
    if state == TaskState::Completed
        && steps
            .iter()
            .any(|step| step.status != PlanStepStatus::Succeeded)
    {
        state = TaskState::Failed;
    }
    Ok(TaskExecutionReport {
        task_id: task_id.into(),
        state,
        steps,
        audit_path: Some(audit_path.to_path_buf()),
    })
}

pub(super) fn audit_agent_loop_task_report(
    session: &ExecutionSession,
    report: &TaskExecutionReport,
) -> Result<()> {
    for step in &report.steps {
        session
            .audit
            .append(session.correlate_audit_event(AuditEvent::new(
                "task_step_result",
                None,
                format!("task step {:?}: {}", step.status, step.skill),
                json!({
                    "correlation_id": session.correlation_id(),
                    "task_id": &report.task_id,
                    "mode": "agent_loop",
                    "step": step,
                }),
            )?))?;
    }
    session
        .audit
        .append(session.correlate_audit_event(AuditEvent::new(
            "task_execution_end",
            None,
            format!("task execution ended: {:?}", report.state),
            json!({
                "correlation_id": session.correlation_id(),
                "task_id": &report.task_id,
                "mode": "agent_loop",
                "state": &report.state,
                "steps": &report.steps,
            }),
        )?))?;
    Ok(())
}

fn agent_loop_tool_result_step(
    result: &AgentLoopToolResult,
    registry: &SkillRegistry,
) -> Result<StepExecutionRecord> {
    let at = now_rfc3339()?;
    let approval_id = approval_id_from_loop_output(&result.output);
    let status =
        if approval_id.is_some() || loop_output_decision(&result.output) == Some("ask_user") {
            PlanStepStatus::WaitingForApproval
        } else if result.ok {
            PlanStepStatus::Succeeded
        } else {
            PlanStepStatus::Failed
        };
    Ok(StepExecutionRecord {
        step_id: Uuid::new_v4().to_string(),
        description: format!("Agent loop tool call: {}", redact_secrets(&result.name)),
        skill: redact_secrets(&result.name),
        risk: registry
            .get(&result.name)
            .map(|skill| skill.risk_level())
            .unwrap_or(RiskLevel::SafeRead),
        status,
        attempts: 1,
        summary: compact_task_summary(&result.summary),
        approval_id,
        started_at: Some(at.clone()),
        completed_at: Some(at),
    })
}

fn agent_loop_final_step(report: &AgentLoopReport) -> Result<StepExecutionRecord> {
    let at = now_rfc3339()?;
    let status = match report.stop_reason {
        AgentLoopStopReason::FinalAnswer => PlanStepStatus::Succeeded,
        AgentLoopStopReason::WaitingForApproval => PlanStepStatus::WaitingForApproval,
        AgentLoopStopReason::PolicyDenied
        | AgentLoopStopReason::GuardrailHalt
        | AgentLoopStopReason::IterationBudget
        | AgentLoopStopReason::Cancelled
        | AgentLoopStopReason::ProviderError
        | AgentLoopStopReason::Compacted
        | AgentLoopStopReason::ToolError
        | AgentLoopStopReason::ContextLimit => PlanStepStatus::Failed,
    };
    let summary = if report.final_content.trim().is_empty() {
        format!("agent loop ended: {:?}", report.stop_reason)
    } else {
        report.final_content.clone()
    };
    Ok(StepExecutionRecord {
        step_id: Uuid::new_v4().to_string(),
        description: "Agent loop final answer.".into(),
        skill: "agent_loop_final".into(),
        risk: RiskLevel::SafeRead,
        status,
        attempts: report.iterations.max(1),
        summary: compact_task_summary(&summary),
        approval_id: None,
        started_at: Some(at.clone()),
        completed_at: Some(at),
    })
}

fn task_state_from_agent_loop(report: &AgentLoopReport) -> TaskState {
    match report.stop_reason {
        AgentLoopStopReason::FinalAnswer => TaskState::Completed,
        AgentLoopStopReason::WaitingForApproval => TaskState::WaitingForApproval,
        AgentLoopStopReason::PolicyDenied
        | AgentLoopStopReason::GuardrailHalt
        | AgentLoopStopReason::IterationBudget
        | AgentLoopStopReason::Cancelled
        | AgentLoopStopReason::ProviderError
        | AgentLoopStopReason::Compacted
        | AgentLoopStopReason::ToolError
        | AgentLoopStopReason::ContextLimit => TaskState::Blocked,
    }
}

fn approval_id_from_loop_output(output: &serde_json::Value) -> Option<String> {
    output
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn loop_output_decision(output: &serde_json::Value) -> Option<&str> {
    output.get("decision").and_then(serde_json::Value::as_str)
}

fn compact_task_summary(summary: &str) -> String {
    let redacted = redact_secrets(summary.trim());
    const MAX_CHARS: usize = 512;
    if redacted.chars().count() <= MAX_CHARS {
        return redacted;
    }
    let mut compact = redacted.chars().take(MAX_CHARS).collect::<String>();
    compact.push_str("...");
    compact
}
