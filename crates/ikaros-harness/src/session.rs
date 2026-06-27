// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    ApprovalLog, ApprovalPolicy, AuditEvent, AuditLog, DefaultPolicyEngine, PolicyEngine,
    PolicyRequest, SandboxProfile, policy::PolicyEvaluation,
};
use ikaros_core::{AgentInstance, ResolvedAgentProfile, Result, ToolResult, redact_secrets};
use ikaros_sandbox::{ExecutionEnv, WorkspaceExecutionEnv};
use ikaros_toolkit::{SkillContext, SkillRegistry, SkillRuntime, SkillSandbox, ToolsetSelection};
use serde_json::json;
use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{Arc, Mutex},
};

mod approvals;
mod skill_execution;

#[derive(Clone)]
pub struct ExecutionSession {
    pub sandbox: SandboxProfile,
    pub policy: Arc<dyn PolicyEngine>,
    pub env: Arc<dyn ExecutionEnv>,
    pub approvals: ApprovalPolicy,
    pub audit: AuditLog,
    pub toolsets: ToolsetSelection,
    pub correlation_id: Option<String>,
    deferred_tool_disclosures: Arc<Mutex<BTreeSet<String>>>,
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
            toolsets: ToolsetSelection::default(),
            correlation_id: None,
            deferred_tool_disclosures: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    pub fn new_with_agent(
        workspace_root: impl Into<PathBuf>,
        audit_dir: impl Into<PathBuf>,
        agent: &ResolvedAgentProfile,
    ) -> Self {
        let mut session = Self::new(workspace_root, audit_dir);
        session.sandbox = session.sandbox.with_agent(agent);
        session.toolsets = toolsets_from_agent_names(&agent.profile.toolsets);
        session
    }

    pub fn new_with_agent_instance(
        workspace_root: impl Into<PathBuf>,
        audit_dir: impl Into<PathBuf>,
        agent: &AgentInstance,
    ) -> Self {
        let mut session = Self::new(workspace_root, audit_dir);
        session.sandbox = session.sandbox.with_agent_instance(agent);
        session.toolsets = toolsets_from_agent_names(&agent.profile.toolsets);
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

    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        let correlation_id = redact_secrets(&correlation_id.into());
        if !correlation_id.trim().is_empty() {
            self.correlation_id = Some(correlation_id);
        }
        self
    }

    pub fn correlation_id(&self) -> Option<&str> {
        self.correlation_id.as_deref()
    }

    pub fn correlate_audit_event(&self, event: AuditEvent) -> AuditEvent {
        match self.correlation_id() {
            Some(correlation_id) => event.with_correlation_id(correlation_id),
            None => event,
        }
    }

    pub fn evaluate(&self, request: &PolicyRequest) -> Result<PolicyEvaluation> {
        let evaluation = self.policy.evaluate(request, &self.sandbox);
        tracing::info!(
            event = "harness_policy_decision",
            action = %request.action,
            risk = ?request.risk,
            decision = ?evaluation.decision,
            reason = %redact_secrets(&evaluation.reason),
            agent = ?self.sandbox.agent.as_ref().map(|agent| agent.name.as_str()),
            agent_id = ?self.sandbox.agent.as_ref().and_then(|agent| agent.agent_id.as_deref()),
            agent_profile = ?self.sandbox.agent.as_ref().map(|agent| agent.profile_name.as_str()),
            agent_mode = ?self.sandbox.agent.as_ref().map(|agent| agent.mode.as_str()),
            correlation_id = ?self.correlation_id(),
            "harness policy decision"
        );
        self.audit
            .append(self.correlate_audit_event(AuditEvent::new(
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
        )?))?;
        Ok(evaluation)
    }

    pub fn disclose_deferred_tool(&self, name: impl Into<String>) {
        self.deferred_tool_disclosures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(name.into());
    }

    pub fn disclose_deferred_tools(&self, names: impl IntoIterator<Item = impl Into<String>>) {
        let mut disclosures = self
            .deferred_tool_disclosures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for name in names {
            disclosures.insert(name.into());
        }
    }

    pub fn is_deferred_tool_disclosed(&self, name: &str) -> bool {
        self.deferred_tool_disclosures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(name)
    }

    pub fn skill_context(&self) -> SkillContext {
        SkillContext::new(
            SkillSandbox::new(self.sandbox.workspace_root.clone()),
            self.env.clone(),
            self.toolsets.clone(),
            Arc::new(self.clone()),
        )
    }

    fn audit_tool_result(&self, skill_name: &str, result: &ToolResult) -> Result<()> {
        self.audit
            .append(self.correlate_audit_event(AuditEvent::new(
                "tool_result",
                None,
                format!("tool result: {skill_name}"),
                json!({
                    "call_id": &result.call_id,
                    "ok": result.ok,
                    "summary": &result.summary,
                    "output": &result.output,
                }),
            )?))
    }
}

fn toolsets_from_agent_names(names: &[String]) -> ToolsetSelection {
    if names.is_empty() {
        return ToolsetSelection::default();
    }
    ToolsetSelection::from_names(names.iter()).unwrap_or_else(|error| {
        panic!("unsupported toolset in agent profile: {error}");
    })
}

impl SkillRuntime for ExecutionSession {
    fn append_audit_event(&self, event: AuditEvent) -> Result<()> {
        self.audit.append(self.correlate_audit_event(event))
    }

    fn audit_path(&self) -> Option<PathBuf> {
        Some(self.audit.path().to_path_buf())
    }

    fn disclose_deferred_tool(&self, name: String) {
        ExecutionSession::disclose_deferred_tool(self, name);
    }

    fn disclose_deferred_tools(&self, names: Vec<String>) {
        ExecutionSession::disclose_deferred_tools(self, names);
    }

    fn is_deferred_tool_disclosed(&self, name: &str) -> bool {
        ExecutionSession::is_deferred_tool_disclosed(self, name)
    }

    fn execute_skill<'a>(
        &'a self,
        registry: &'a SkillRegistry,
        name: &'a str,
        input: serde_json::Value,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<ToolResult>> + Send + 'a>> {
        Box::pin(async move { ExecutionSession::execute_skill(self, registry, name, input).await })
    }
}
