// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{AgentInstance, PolicyDecision, ResolvedAgentProfile, RiskLevel};
use ikaros_toolkit::PolicyRequest;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SandboxProfile {
    pub workspace_root: PathBuf,
    pub protected_paths: Vec<PathBuf>,
    pub dry_run: bool,
    pub explain: bool,
    pub agent: Option<AgentPolicyOverlay>,
}

impl SandboxProfile {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            protected_paths: vec![PathBuf::from(".temp")],
            dry_run: false,
            explain: true,
            agent: None,
        }
    }

    pub fn with_agent(mut self, agent: &ResolvedAgentProfile) -> Self {
        self.agent = Some(AgentPolicyOverlay::from(agent));
        self
    }

    pub fn with_agent_instance(mut self, agent: &AgentInstance) -> Self {
        self.agent = Some(AgentPolicyOverlay::from(agent));
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPolicyOverlay {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    pub name: String,
    pub profile_name: String,
    pub mode: String,
    pub workspace_writes: PolicyDecision,
    pub shell: PolicyDecision,
    pub network: PolicyDecision,
}

impl From<&ResolvedAgentProfile> for AgentPolicyOverlay {
    fn from(agent: &ResolvedAgentProfile) -> Self {
        Self {
            agent_id: None,
            name: agent.name.clone(),
            profile_name: agent.name.clone(),
            mode: agent.mode().as_str().into(),
            workspace_writes: agent.profile.workspace_writes.to_policy_decision(),
            shell: agent.profile.shell.to_policy_decision(),
            network: agent.profile.network.to_policy_decision(),
        }
    }
}

impl From<&AgentInstance> for AgentPolicyOverlay {
    fn from(agent: &AgentInstance) -> Self {
        Self {
            agent_id: Some(agent.agent_id.clone()),
            name: agent.agent_id.clone(),
            profile_name: agent.profile_name.clone(),
            mode: agent.profile.mode.as_str().into(),
            workspace_writes: agent.profile.workspace_writes.to_policy_decision(),
            shell: agent.profile.shell.to_policy_decision(),
            network: combine_policy_decisions([
                agent.profile.network.to_policy_decision(),
                agent.auth_scope.allow_network.to_policy_decision(),
            ]),
        }
    }
}

fn combine_policy_decisions(decisions: impl IntoIterator<Item = PolicyDecision>) -> PolicyDecision {
    let mut combined = PolicyDecision::Allow;
    for decision in decisions {
        match decision {
            PolicyDecision::Deny => return PolicyDecision::Deny,
            PolicyDecision::AskUser => combined = PolicyDecision::AskUser,
            PolicyDecision::Allow => {}
        }
    }
    combined
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScopedPermission {
    pub root: PathBuf,
    pub risk: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CapabilityToken {
    pub id: String,
    pub permission: ScopedPermission,
    pub expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PolicyEvaluation {
    pub decision: PolicyDecision,
    pub reason: String,
}

pub trait PolicyEngine: Send + Sync {
    fn evaluate(&self, request: &PolicyRequest, sandbox: &SandboxProfile) -> PolicyEvaluation;
}
