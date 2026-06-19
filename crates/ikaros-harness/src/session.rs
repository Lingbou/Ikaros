// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    ApprovalLog, ApprovalPolicy, AuditEvent, AuditLog, DefaultPolicyEngine, PolicyEngine,
    PolicyRequest, SandboxProfile,
    execution_env::{ExecutionEnv, WorkspaceExecutionEnv},
    policy::PolicyEvaluation,
};
use ikaros_core::{AgentInstance, ResolvedAgentProfile, Result, ToolResult};
use serde_json::json;
use std::{path::PathBuf, sync::Arc};

mod approvals;
mod skill_execution;

#[derive(Clone)]
pub struct ExecutionSession {
    pub sandbox: SandboxProfile,
    pub policy: Arc<dyn PolicyEngine>,
    pub env: Arc<dyn ExecutionEnv>,
    pub approvals: ApprovalPolicy,
    pub audit: AuditLog,
}

impl ExecutionSession {
    pub fn new(workspace_root: impl Into<PathBuf>, audit_dir: impl Into<PathBuf>) -> Self {
        let audit_dir = audit_dir.into();
        let workspace_root = workspace_root.into();
        Self {
            sandbox: SandboxProfile::new(&workspace_root),
            policy: Arc::new(DefaultPolicyEngine),
            env: Arc::new(WorkspaceExecutionEnv::local(workspace_root)),
            approvals: ApprovalPolicy::with_log(ApprovalLog::new(&audit_dir)),
            audit: AuditLog::new(audit_dir),
        }
    }

    pub fn new_with_agent(
        workspace_root: impl Into<PathBuf>,
        audit_dir: impl Into<PathBuf>,
        agent: &ResolvedAgentProfile,
    ) -> Self {
        let mut session = Self::new(workspace_root, audit_dir);
        session.sandbox = session.sandbox.with_agent(agent);
        session
    }

    pub fn new_with_agent_instance(
        workspace_root: impl Into<PathBuf>,
        audit_dir: impl Into<PathBuf>,
        agent: &AgentInstance,
    ) -> Self {
        let mut session = Self::new(workspace_root, audit_dir);
        session.sandbox = session.sandbox.with_agent_instance(agent);
        session
    }

    pub fn with_dry_run(mut self, dry_run: bool) -> Self {
        self.sandbox.dry_run = dry_run;
        self
    }

    pub fn with_explain(mut self, explain: bool) -> Self {
        self.sandbox.explain = explain;
        self
    }

    pub fn with_execution_env(mut self, env: Arc<dyn ExecutionEnv>) -> Self {
        self.env = env;
        self
    }

    pub fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyEvaluation> {
        let evaluation = self.policy.evaluate(request, &self.sandbox);
        self.audit.append(AuditEvent::new(
            "policy_decision",
            Some(evaluation.decision.clone()),
            &evaluation.reason,
            json!({
                "action": request.action,
                "risk": request.risk,
                "path": request.path,
                "command": request.command,
                "agent": self.sandbox.agent.as_ref().map(|agent| &agent.name),
                "agent_id": self.sandbox.agent.as_ref().and_then(|agent| agent.agent_id.as_ref()),
                "agent_profile": self.sandbox.agent.as_ref().map(|agent| &agent.profile_name),
                "agent_mode": self.sandbox.agent.as_ref().map(|agent| &agent.mode),
            }),
        )?)?;
        Ok(evaluation)
    }

    fn audit_tool_result(&self, skill_name: &str, result: &ToolResult) -> Result<()> {
        self.audit.append(AuditEvent::new(
            "tool_result",
            None,
            format!("tool result: {skill_name}"),
            json!({
                "call_id": &result.call_id,
                "ok": result.ok,
                "summary": &result.summary,
                "output": &result.output,
            }),
        )?)
    }
}
