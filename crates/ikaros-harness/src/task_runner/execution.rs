// SPDX-License-Identifier: GPL-3.0-only

use super::{
    cancellation::CancellationToken,
    record::{ExecutablePlanStep, PlanStepStatus, StepExecutionRecord, TaskExecutionReport},
    types::ExecutionOptions,
};
use crate::{
    AuditEvent, ExecutionSession, GuardrailDecision, GuardrailObservation, GuardrailSignal,
    GuardrailState, SkillRegistry,
};
use ikaros_core::{Result, TaskState, ToolResult, redact_secrets};
use serde_json::json;
use tokio::time::{Duration, sleep, timeout};

enum StepAttemptOutcome {
    Tool(ToolResult),
    Failed(String),
    TimedOut(String),
}

impl ExecutionSession {
    pub async fn execute_task_steps(
        &self,
        registry: &SkillRegistry,
        task_id: impl Into<String>,
        steps: Vec<ExecutablePlanStep>,
        options: ExecutionOptions,
        cancellation: CancellationToken,
    ) -> Result<TaskExecutionReport> {
        let task_id = task_id.into();
        let mut report = TaskExecutionReport {
            task_id: task_id.clone(),
            state: TaskState::Running,
            steps: steps.iter().map(StepExecutionRecord::pending).collect(),
            audit_path: Some(self.audit.path().to_path_buf()),
        };
        let mut guardrails = GuardrailState::default();
        self.audit
            .append(self.correlate_audit_event(AuditEvent::new(
                "task_execution_start",
                None,
                format!("task execution started: {task_id}"),
                json!({
                    "correlation_id": self.correlation_id(),
                    "task_id": &task_id,
                    "step_count": steps.len(),
                    "dry_run": self.sandbox.dry_run,
                    "guardrails": &options.guardrails,
                }),
            )?))?;

        for (index, step) in steps.iter().enumerate() {
            if cancellation.is_cancelled() {
                report.steps[index].complete(
                    PlanStepStatus::Cancelled,
                    "task cancelled before step started",
                    None,
                )?;
                report.state = TaskState::Cancelled;
                self.audit_task_step_result(&task_id, &report.steps[index])?;
                self.audit_task_execution_end(&report)?;
                return Ok(report);
            }

            report.steps[index].start()?;
            self.audit
                .append(self.correlate_audit_event(AuditEvent::new(
                    "task_step_start",
                    None,
                    format!("task step started: {}", step.skill),
                    json!({
                        "correlation_id": self.correlation_id(),
                        "task_id": &task_id,
                        "step_id": &step.id,
                        "skill": &step.skill,
                        "risk": &step.risk,
                    }),
                )?))?;

            let max_attempts = u32::from(options.max_retries) + 1;
            for attempt in 1..=max_attempts {
                report.steps[index].attempts = attempt;
                if cancellation.is_cancelled() {
                    report.steps[index].complete(
                        PlanStepStatus::Cancelled,
                        "task cancelled before step execution",
                        None,
                    )?;
                    report.state = TaskState::Cancelled;
                    self.audit_task_step_result(&task_id, &report.steps[index])?;
                    self.audit_task_execution_end(&report)?;
                    return Ok(report);
                }

                match self.execute_step_attempt(registry, step, &options).await {
                    StepAttemptOutcome::Tool(result) if result.ok => {
                        let observation = GuardrailObservation::tool(
                            &step.skill,
                            result.ok,
                            &result.summary,
                            &result.output,
                        );
                        match guardrails.observe(&options.guardrails, &observation) {
                            GuardrailDecision::Continue => {}
                            GuardrailDecision::Warn(signal) => {
                                self.audit_guardrail_signal(&task_id, step, &signal, false)?;
                            }
                            GuardrailDecision::Halt(signal) => {
                                self.audit_guardrail_signal(&task_id, step, &signal, true)?;
                                report.steps[index].complete(
                                    PlanStepStatus::Failed,
                                    format!("guardrail halted: {}", signal.message()),
                                    None,
                                )?;
                                report.state = TaskState::Blocked;
                                self.audit_task_step_result(&task_id, &report.steps[index])?;
                                self.audit_task_execution_end(&report)?;
                                return Ok(report);
                            }
                        }
                        report.steps[index].complete(
                            PlanStepStatus::Succeeded,
                            result.summary,
                            None,
                        )?;
                        self.audit_task_step_result(&task_id, &report.steps[index])?;
                        break;
                    }
                    StepAttemptOutcome::Tool(result) => {
                        let approval_id = approval_id_from_result(&result);
                        let status = if approval_id.is_some()
                            || result_decision(&result) == Some("ask_user")
                        {
                            PlanStepStatus::WaitingForApproval
                        } else {
                            PlanStepStatus::Failed
                        };
                        if should_observe_tool_failure(&result) {
                            let observation = GuardrailObservation::tool(
                                &step.skill,
                                result.ok,
                                &result.summary,
                                &result.output,
                            );
                            match guardrails.observe(&options.guardrails, &observation) {
                                GuardrailDecision::Continue => {}
                                GuardrailDecision::Warn(signal) => {
                                    self.audit_guardrail_signal(&task_id, step, &signal, false)?;
                                }
                                GuardrailDecision::Halt(signal) => {
                                    self.audit_guardrail_signal(&task_id, step, &signal, true)?;
                                    report.steps[index].complete(
                                        PlanStepStatus::Failed,
                                        format!("guardrail halted: {}", signal.message()),
                                        None,
                                    )?;
                                    report.state = TaskState::Blocked;
                                    self.audit_task_step_result(&task_id, &report.steps[index])?;
                                    self.audit_task_execution_end(&report)?;
                                    return Ok(report);
                                }
                            }
                        }
                        report.steps[index].complete(
                            status.clone(),
                            result.summary,
                            approval_id,
                        )?;
                        report.state = match status {
                            PlanStepStatus::WaitingForApproval => TaskState::WaitingForApproval,
                            _ => TaskState::Failed,
                        };
                        self.audit_task_step_result(&task_id, &report.steps[index])?;
                        self.audit_task_execution_end(&report)?;
                        return Ok(report);
                    }
                    StepAttemptOutcome::Failed(summary) | StepAttemptOutcome::TimedOut(summary) => {
                        let observation = GuardrailObservation::failure(&step.skill, &summary);
                        match guardrails.observe(&options.guardrails, &observation) {
                            GuardrailDecision::Continue => {}
                            GuardrailDecision::Warn(signal) => {
                                self.audit_guardrail_signal(&task_id, step, &signal, false)?;
                            }
                            GuardrailDecision::Halt(signal) => {
                                self.audit_guardrail_signal(&task_id, step, &signal, true)?;
                                report.steps[index].complete(
                                    PlanStepStatus::Failed,
                                    format!("guardrail halted: {}", signal.message()),
                                    None,
                                )?;
                                report.state = TaskState::Blocked;
                                self.audit_task_step_result(&task_id, &report.steps[index])?;
                                self.audit_task_execution_end(&report)?;
                                return Ok(report);
                            }
                        }
                        if attempt < max_attempts {
                            report.steps[index].summary = redact_secrets(&format!(
                                "attempt {attempt} failed: {summary}; retrying"
                            ));
                            self.audit
                                .append(self.correlate_audit_event(AuditEvent::new(
                                    "task_step_retry",
                                    None,
                                    format!("task step retry: {}", step.skill),
                                    json!({
                                        "correlation_id": self.correlation_id(),
                                        "task_id": &task_id,
                                        "step_id": &step.id,
                                        "skill": &step.skill,
                                        "attempt": attempt,
                                        "max_attempts": max_attempts,
                                        "summary": &report.steps[index].summary,
                                    }),
                                )?))?;
                            if options.retry_delay_ms > 0 {
                                sleep(Duration::from_millis(options.retry_delay_ms)).await;
                            }
                            continue;
                        }
                        report.steps[index].complete(PlanStepStatus::Failed, summary, None)?;
                        report.state = TaskState::Failed;
                        self.audit_task_step_result(&task_id, &report.steps[index])?;
                        self.audit_task_execution_end(&report)?;
                        return Ok(report);
                    }
                }
            }
        }

        report.state = TaskState::Completed;
        self.audit_task_execution_end(&report)?;
        Ok(report)
    }

    async fn execute_step_attempt(
        &self,
        registry: &SkillRegistry,
        step: &ExecutablePlanStep,
        options: &ExecutionOptions,
    ) -> StepAttemptOutcome {
        let future = self.execute_skill(registry, &step.skill, step.input.clone());
        match options.timeout_ms {
            Some(timeout_ms) if timeout_ms > 0 => {
                match timeout(Duration::from_millis(timeout_ms), future).await {
                    Ok(Ok(result)) => StepAttemptOutcome::Tool(result),
                    Ok(Err(error)) => {
                        StepAttemptOutcome::Failed(redact_secrets(&error.to_string()))
                    }
                    Err(_) => StepAttemptOutcome::TimedOut(format!(
                        "step timed out after {timeout_ms} ms"
                    )),
                }
            }
            _ => match future.await {
                Ok(result) => StepAttemptOutcome::Tool(result),
                Err(error) => StepAttemptOutcome::Failed(redact_secrets(&error.to_string())),
            },
        }
    }

    fn audit_task_step_result(&self, task_id: &str, record: &StepExecutionRecord) -> Result<()> {
        self.audit
            .append(self.correlate_audit_event(AuditEvent::new(
                "task_step_result",
                None,
                format!("task step {:?}: {}", record.status, record.skill),
                json!({
                    "correlation_id": self.correlation_id(),
                    "task_id": task_id,
                    "step": record,
                }),
            )?))
    }

    fn audit_guardrail_signal(
        &self,
        task_id: &str,
        step: &ExecutablePlanStep,
        signal: &GuardrailSignal,
        halted: bool,
    ) -> Result<()> {
        let kind = if halted {
            "task_guardrail_halt"
        } else {
            "task_guardrail_warning"
        };
        self.audit
            .append(self.correlate_audit_event(AuditEvent::new(
                kind,
                None,
                signal.message(),
                json!({
                    "correlation_id": self.correlation_id(),
                    "task_id": task_id,
                    "step_id": &step.id,
                    "skill": &step.skill,
                    "signal": signal,
                    "halted": halted,
                }),
            )?))
    }

    fn audit_task_execution_end(&self, report: &TaskExecutionReport) -> Result<()> {
        self.audit
            .append(self.correlate_audit_event(AuditEvent::new(
                "task_execution_end",
                None,
                format!("task execution ended: {:?}", report.state),
                json!({
                    "correlation_id": self.correlation_id(),
                    "task_id": &report.task_id,
                    "state": &report.state,
                    "steps": &report.steps,
                }),
            )?))
    }
}

fn approval_id_from_result(result: &ToolResult) -> Option<String> {
    result
        .output
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn result_decision(result: &ToolResult) -> Option<&str> {
    result
        .output
        .get("decision")
        .and_then(serde_json::Value::as_str)
}

fn should_observe_tool_failure(result: &ToolResult) -> bool {
    approval_id_from_result(result).is_none() && result_decision(result).is_none()
}
