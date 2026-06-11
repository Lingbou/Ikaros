// SPDX-License-Identifier: GPL-3.0-only

use super::ExecutionSession;
use crate::{
    ApprovalRecord, ApprovalStatus, AuditEvent, SkillRegistry, policy::canonicalize_path_for_policy,
};
use ikaros_core::{IkarosError, PolicyDecision, Result, ToolResult};
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
        self.audit.append(AuditEvent::new(
            "approval_decision",
            None,
            format!("approval {} is now {:?}", approval_id, record.status),
            json!({
                "approval_id": approval_id,
                "status": record.status,
                "tool": record.request.call.name,
            }),
        )?)?;
        Ok(record)
    }

    pub async fn execute_approved_skill(
        &self,
        registry: &SkillRegistry,
        approval_id: &str,
    ) -> Result<ToolResult> {
        let record = self
            .approvals
            .get(approval_id)?
            .ok_or_else(|| IkarosError::Message(format!("approval not found: {approval_id}")))?;
        if record.status != ApprovalStatus::Approved {
            return Err(IkarosError::Message(format!(
                "approval {approval_id} is {:?}, not approved",
                record.status
            )));
        }
        let execution_request = self
            .approvals
            .execution_request(approval_id)?
            .unwrap_or_else(|| record.request.clone());
        if let Some(root) = &execution_request.workspace_root {
            let expected = canonicalize_path_for_policy(root);
            let actual = canonicalize_path_for_policy(&self.sandbox.workspace_root);
            if expected != actual {
                return Err(IkarosError::Message(format!(
                    "approval {approval_id} was created for workspace {}, current workspace is {}",
                    expected.display(),
                    actual.display()
                )));
            }
        }
        let call = execution_request.call.clone();
        let skill = registry.get(&call.name).ok_or_else(|| {
            IkarosError::Message(format!(
                "skill not found for approval {}: {}",
                approval_id, call.name
            ))
        })?;
        let current_request = skill.policy_request(&call.input, &self.sandbox.workspace_root);
        self.audit.append(AuditEvent::new(
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
        )?)?;
        let evaluation = self.evaluate(&current_request)?;
        if evaluation.decision == PolicyDecision::Deny {
            return Err(IkarosError::Message(format!(
                "approval {approval_id} is no longer allowed by current policy: {}",
                evaluation.reason
            )));
        }
        self.audit.append(AuditEvent::new(
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
        )?)?;
        let result = if self.sandbox.dry_run {
            ToolResult {
                call_id: call.id.clone(),
                ok: true,
                output: json!({"dry_run": true, "approval_id": approval_id}),
                summary: format!("dry-run approved {}", skill.name()),
            }
        } else {
            match self
                .env
                .execute_skill(skill.clone(), call.input.clone(), self)
                .await
            {
                Ok(output) => ToolResult {
                    call_id: call.id.clone(),
                    ok: true,
                    summary: output.summary,
                    output: output.output,
                },
                Err(error) => {
                    let result = ToolResult {
                        call_id: call.id.clone(),
                        ok: false,
                        output: json!({"error": error.to_string(), "approval_id": approval_id}),
                        summary: format!("approved skill {} failed", skill.name()),
                    };
                    self.audit_tool_result(skill.name(), &result)?;
                    self.approvals.mark_executed(approval_id, result)?;
                    return Err(error);
                }
            }
        };
        self.audit_tool_result(skill.name(), &result)?;
        self.approvals.mark_executed(approval_id, result.clone())?;
        Ok(result)
    }
}
