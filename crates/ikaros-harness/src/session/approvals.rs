// SPDX-License-Identifier: GPL-3.0-only

use super::ExecutionSession;
use crate::{
    ApprovalRecord, ApprovalStatus, AuditEvent, SkillRegistry, policy::canonicalize_path_for_policy,
};
use ikaros_core::{IkarosError, PolicyDecision, Result, ToolResult, redact_secrets};
use serde_json::json;

impl ExecutionSession {
    pub fn approval_records(&self) -> Result<Vec<ApprovalRecord>> {
        self.approvals.records()
    }

    pub fn pending_approvals(&self) -> Result<Vec<ApprovalRecord>> {
        Ok(self
            .approval_records()?
            .into_iter()
            .filter(|record| record.status == ApprovalStatus::Pending)
            .collect())
    }

    pub fn decide_approval(
        &self,
        approval_id: &str,
        status: ApprovalStatus,
        note: Option<String>,
    ) -> Result<ApprovalRecord> {
        let record = self.approvals.decide(approval_id, status, note)?;
        tracing::info!(
            event = "harness_approval_decided",
            approval_id,
            status = ?record.status,
            tool = %record.request.call.name,
            correlation_id = ?self.correlation_id(),
            "harness approval decision recorded"
        );
        self.audit
            .append(self.correlate_audit_event(AuditEvent::new(
                "approval_decision",
                None,
                format!("approval {} is now {:?}", approval_id, record.status),
                json!({
                    "approval_id": approval_id,
                    "status": record.status,
                    "tool": record.request.call.name,
                }),
            )?))?;
        Ok(record)
    }

    pub async fn execute_approved_skill(
        &self,
        registry: &SkillRegistry,
        approval_id: &str,
    ) -> Result<ToolResult> {
        tracing::info!(
            event = "harness_approval_execution_requested",
            approval_id,
            correlation_id = ?self.correlation_id(),
            "harness approved tool execution requested"
        );
        let record = self
            .approvals
            .get(approval_id)?
            .ok_or_else(|| IkarosError::Message(format!("approval not found: {approval_id}")))?;
        if record.status != ApprovalStatus::Approved {
            tracing::warn!(
                event = "harness_approval_execution_rejected",
                approval_id,
                status = ?record.status,
                correlation_id = ?self.correlation_id(),
                "harness approval is not approved"
            );
            return Err(IkarosError::Message(format!(
                "approval {approval_id} is {:?}, not approved",
                record.status
            )));
        }
        let execution_request = self
            .approvals
            .execution_request(approval_id)?
            .ok_or_else(|| {
                tracing::warn!(
                    event = "harness_approval_execution_missing_request",
                    approval_id,
                    correlation_id = ?self.correlation_id(),
                    "harness approval execution request missing"
                );
                IkarosError::Message(format!(
                    "approval {approval_id} is missing its execution request; legacy approval replay is not supported"
                ))
            })?;
        if let Some(root) = &execution_request.workspace_root {
            let expected = canonicalize_path_for_policy(root);
            let actual = canonicalize_path_for_policy(&self.sandbox.workspace_root);
            if expected != actual {
                tracing::warn!(
                    event = "harness_approval_workspace_mismatch",
                    approval_id,
                    expected = %expected.display(),
                    actual = %actual.display(),
                    correlation_id = ?self.correlation_id(),
                    "harness approval workspace mismatch"
                );
                return Err(IkarosError::Message(format!(
                    "approval {approval_id} was created for workspace {}, current workspace is {}",
                    expected.display(),
                    actual.display()
                )));
            }
        }
        let call = execution_request.call.clone();
        let skill = registry.get(&call.name).ok_or_else(|| {
            tracing::warn!(
                event = "harness_approval_skill_missing",
                approval_id,
                call_id = %call.id,
                skill = %call.name,
                correlation_id = ?self.correlation_id(),
                "harness approval skill missing"
            );
            IkarosError::Message(format!(
                "skill not found for approval {}: {}",
                approval_id, call.name
            ))
        })?;
        let current_request = skill.policy_request(&call.input, &self.sandbox.workspace_root);
        tracing::info!(
            event = "harness_approval_revalidation_started",
            approval_id,
            call_id = %call.id,
            skill = %call.name,
            risk = ?call.risk,
            correlation_id = ?self.correlation_id(),
            "harness approval revalidation started"
        );
        self.audit
            .append(self.correlate_audit_event(AuditEvent::new(
                "tool_call",
                None,
                format!("approved tool call requested: {}", skill.name()),
                json!({
                    "approval_id": approval_id,
                    "call_id": &call.id,
                    "name": &call.name,
                    "risk": &call.risk,
                    "input": &call.input,
                }),
            )?))?;
        let evaluation = self.evaluate(&current_request)?;
        if evaluation.decision == PolicyDecision::Deny {
            tracing::warn!(
                event = "harness_approval_revalidation_denied",
                approval_id,
                call_id = %call.id,
                skill = %call.name,
                risk = ?call.risk,
                reason = %redact_secrets(&evaluation.reason),
                correlation_id = ?self.correlation_id(),
                "harness approval revalidation denied"
            );
            return Err(IkarosError::Message(format!(
                "approval {approval_id} is no longer allowed by current policy: {}",
                evaluation.reason
            )));
        }
        tracing::info!(
            event = "harness_approval_revalidation_completed",
            approval_id,
            call_id = %call.id,
            skill = %call.name,
            risk = ?call.risk,
            decision = ?evaluation.decision,
            correlation_id = ?self.correlation_id(),
            "harness approval revalidation completed"
        );
        self.audit
            .append(self.correlate_audit_event(AuditEvent::new(
                "policy_decision",
                Some(PolicyDecision::Allow),
                format!("approval {approval_id} accepted"),
                json!({
                    "approval_id": approval_id,
                    "action": &call.name,
                    "risk": &call.risk,
                    "revalidated_decision": evaluation.decision,
                    "revalidated_reason": evaluation.reason,
                }),
            )?))?;
        let result = if self.sandbox.dry_run {
            tracing::info!(
                event = "harness_approval_execution_dry_run",
                approval_id,
                call_id = %call.id,
                skill = %call.name,
                risk = ?call.risk,
                correlation_id = ?self.correlation_id(),
                "harness approved tool skipped by dry-run"
            );
            ToolResult {
                call_id: call.id.clone(),
                ok: true,
                output: json!({"dry_run": true, "approval_id": approval_id}),
                summary: format!("dry-run approved {}", skill.name()),
            }
        } else {
            tracing::info!(
                event = "harness_approval_execution_started",
                approval_id,
                call_id = %call.id,
                skill = %call.name,
                risk = ?call.risk,
                correlation_id = ?self.correlation_id(),
                "harness approved tool execution started"
            );
            let context = self.skill_context();
            match self
                .env
                .execute_skill(skill.clone(), call.input.clone(), context)
                .await
            {
                Ok(output) => {
                    tracing::info!(
                        event = "harness_approval_execution_completed",
                        approval_id,
                        call_id = %call.id,
                        skill = %call.name,
                        risk = ?call.risk,
                        ok = true,
                        correlation_id = ?self.correlation_id(),
                        "harness approved tool execution completed"
                    );
                    ToolResult {
                        call_id: call.id.clone(),
                        ok: true,
                        summary: output.summary,
                        output: output.output,
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        event = "harness_approval_execution_failed",
                        approval_id,
                        call_id = %call.id,
                        skill = %call.name,
                        risk = ?call.risk,
                        error = %redact_secrets(&error.to_string()),
                        correlation_id = ?self.correlation_id(),
                        "harness approved tool execution failed"
                    );
                    let result = ToolResult {
                        call_id: call.id.clone(),
                        ok: false,
                        output: json!({"error": error.to_string(), "approval_id": approval_id}),
                        summary: format!("approved skill {} failed", skill.name()),
                    };
                    self.audit_tool_result(skill.name(), &result)?;
                    return Err(error);
                }
            }
        };
        self.audit_tool_result(skill.name(), &result)?;
        self.approvals.mark_executed(approval_id, result.clone())?;
        tracing::info!(
            event = "harness_approval_marked_executed",
            approval_id,
            call_id = %call.id,
            skill = %call.name,
            ok = result.ok,
            correlation_id = ?self.correlation_id(),
            "harness approval marked executed"
        );
        Ok(result)
    }
}
