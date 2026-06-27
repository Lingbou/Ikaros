// SPDX-License-Identifier: GPL-3.0-only

use super::ExecutionSession;
use crate::{AuditEvent, Skill, SkillRegistry};
use ikaros_core::{
    IkarosError, PolicyDecision, Result, RiskLevel, ToolCall, ToolResult, redact_secrets,
};
use serde_json::json;
use std::sync::Arc;

impl ExecutionSession {
    pub async fn execute_skill(
        &self,
        registry: &SkillRegistry,
        name: &str,
        input: serde_json::Value,
    ) -> Result<ToolResult> {
        self.execute_skill_internal(registry, name, input.clone(), input)
            .await
    }

    pub async fn execute_read_skill_with_audit_input(
        &self,
        registry: &SkillRegistry,
        name: &str,
        input: serde_json::Value,
        audit_input: serde_json::Value,
    ) -> Result<ToolResult> {
        let skill = registry
            .get(name)
            .ok_or_else(|| IkarosError::Message(format!("skill not found: {name}")))?;
        if skill.risk_level() != RiskLevel::SafeRead {
            return Err(IkarosError::Message(format!(
                "redacted audit input is only supported for SafeRead skills: {name}"
            )));
        }
        self.execute_skill_internal_with_skill(skill, input, audit_input)
            .await
    }

    async fn execute_skill_internal(
        &self,
        registry: &SkillRegistry,
        name: &str,
        input: serde_json::Value,
        audit_input: serde_json::Value,
    ) -> Result<ToolResult> {
        let skill = registry
            .get(name)
            .ok_or_else(|| IkarosError::Message(format!("skill not found: {name}")))?;
        self.execute_skill_internal_with_skill(skill, input, audit_input)
            .await
    }

    async fn execute_skill_internal_with_skill(
        &self,
        skill: Arc<dyn Skill>,
        input: serde_json::Value,
        audit_input: serde_json::Value,
    ) -> Result<ToolResult> {
        let audit_input_redacted = audit_input != input;
        let request = skill.policy_request(&input, &self.sandbox.workspace_root);
        let call = ToolCall::new(skill.name(), request.risk.clone(), input.clone());
        let call_id = call.id.clone();
        let skill_name = call.name.clone();
        let risk = call.risk.clone();
        tracing::info!(
            event = "harness_tool_requested",
            call_id = %call_id,
            skill = %skill_name,
            risk = ?risk,
            audit_input_redacted,
            correlation_id = ?self.correlation_id(),
            "harness tool requested"
        );
        self.audit
            .append(self.correlate_audit_event(AuditEvent::new(
                "tool_call",
                None,
                format!("tool call requested: {}", skill.name()),
                json!({
                    "call_id": &call.id,
                    "name": &call.name,
                    "risk": &call.risk,
                    "input": audit_input,
                    "audit_input_redacted": audit_input_redacted,
                }),
            )?))?;
        let evaluation = self.evaluate(&request)?;
        tracing::info!(
            event = "harness_tool_policy",
            call_id = %call_id,
            skill = %skill_name,
            risk = ?risk,
            decision = ?evaluation.decision,
            reason = %redact_secrets(&evaluation.reason),
            correlation_id = ?self.correlation_id(),
            "harness tool policy decision"
        );
        let result = match evaluation.decision {
            PolicyDecision::Allow => {
                if self.sandbox.dry_run {
                    tracing::info!(
                        event = "harness_tool_dry_run",
                        call_id = %call_id,
                        skill = %skill_name,
                        risk = ?risk,
                        correlation_id = ?self.correlation_id(),
                        "harness tool skipped by dry-run"
                    );
                    ToolResult {
                        call_id: call.id,
                        ok: true,
                        output: json!({"dry_run": true}),
                        summary: format!("dry-run allowed {}", skill.name()),
                    }
                } else {
                    tracing::info!(
                        event = "harness_tool_started",
                        call_id = %call_id,
                        skill = %skill_name,
                        risk = ?risk,
                        correlation_id = ?self.correlation_id(),
                        "harness tool execution started"
                    );
                    let context = self.skill_context();
                    match self.env.execute_skill(skill.clone(), input, context).await {
                        Ok(output) => {
                            tracing::info!(
                                event = "harness_tool_completed",
                                call_id = %call_id,
                                skill = %skill_name,
                                risk = ?risk,
                                ok = true,
                                correlation_id = ?self.correlation_id(),
                                "harness tool execution completed"
                            );
                            ToolResult {
                                call_id: call.id,
                                ok: true,
                                summary: output.summary,
                                output: output.output,
                            }
                        }
                        Err(error) => {
                            tracing::warn!(
                                event = "harness_tool_failed",
                                call_id = %call_id,
                                skill = %skill_name,
                                risk = ?risk,
                                error = %redact_secrets(&error.to_string()),
                                correlation_id = ?self.correlation_id(),
                                "harness tool execution failed"
                            );
                            let result = ToolResult {
                                call_id: call.id,
                                ok: false,
                                output: json!({"error": error.to_string()}),
                                summary: format!("skill {} failed", skill.name()),
                            };
                            self.audit_tool_result(skill.name(), &result)?;
                            return Err(error);
                        }
                    }
                }
            }
            PolicyDecision::AskUser => {
                let approval_context = skill.approval_context(&input, &self.sandbox.workspace_root);
                let approval = self.approvals.enqueue(
                    call.clone(),
                    evaluation.reason.clone(),
                    self.sandbox.workspace_root.clone(),
                    approval_context.clone(),
                )?;
                tracing::info!(
                    event = "harness_tool_approval_queued",
                    call_id = %call_id,
                    skill = %skill_name,
                    risk = ?risk,
                    approval_id = %approval.id,
                    reason = %redact_secrets(&evaluation.reason),
                    correlation_id = ?self.correlation_id(),
                    "harness tool approval queued"
                );
                let mut output = json!({"approval_id": approval.id, "decision": "ask_user"});
                if let Some(context) = approval_context {
                    output["approval_context"] = context;
                }
                ToolResult {
                    call_id: call.id,
                    ok: false,
                    output,
                    summary: evaluation.reason,
                }
            }
            PolicyDecision::Deny => {
                tracing::warn!(
                    event = "harness_tool_denied",
                    call_id = %call_id,
                    skill = %skill_name,
                    risk = ?risk,
                    reason = %redact_secrets(&evaluation.reason),
                    correlation_id = ?self.correlation_id(),
                    "harness tool denied"
                );
                ToolResult {
                    call_id: call.id,
                    ok: false,
                    output: json!({"decision": "deny"}),
                    summary: evaluation.reason,
                }
            }
        };
        self.audit_tool_result(skill.name(), &result)?;
        Ok(result)
    }
}
