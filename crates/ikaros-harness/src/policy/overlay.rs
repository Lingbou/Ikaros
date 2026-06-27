// SPDX-License-Identifier: GPL-3.0-only

use super::types::{AgentPolicyOverlay, PolicyEvaluation, SandboxProfile};
use ikaros_core::{PolicyDecision, RiskLevel};
use ikaros_tools::PolicyRequest;

pub(super) fn evaluate_agent_policy_overlay(
    request: &PolicyRequest,
    sandbox: &SandboxProfile,
) -> Option<PolicyEvaluation> {
    let agent = sandbox.agent.as_ref()?;
    let decision = match request.risk {
        RiskLevel::SafeRead
        | RiskLevel::Destructive
        | RiskLevel::SecretAccess
        | RiskLevel::SelfModify => return None,
        RiskLevel::LocalWrite => agent.workspace_writes.clone(),
        RiskLevel::ShellRead => agent.shell.clone(),
        RiskLevel::ShellWrite => {
            combine_policy_decisions([agent.shell.clone(), agent.workspace_writes.clone()])
        }
        RiskLevel::DatabaseWrite => match &agent.workspace_writes {
            PolicyDecision::Deny => PolicyDecision::Deny,
            PolicyDecision::Allow => PolicyDecision::Allow,
            PolicyDecision::AskUser => return None,
        },
        RiskLevel::Network | RiskLevel::RemoteServer => {
            if request.is_write {
                combine_policy_decisions([agent.network.clone(), agent.workspace_writes.clone()])
            } else {
                agent.network.clone()
            }
        }
    };
    Some(agent_policy_evaluation(agent, request, decision))
}

pub(super) fn profile_workspace_write_decision(
    request: &PolicyRequest,
    sandbox: &SandboxProfile,
) -> Option<PolicyDecision> {
    let agent = sandbox.agent.as_ref()?;
    if request.is_write {
        Some(agent.workspace_writes.clone())
    } else {
        None
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

fn agent_policy_evaluation(
    agent: &AgentPolicyOverlay,
    request: &PolicyRequest,
    decision: PolicyDecision,
) -> PolicyEvaluation {
    let action = match decision {
        PolicyDecision::Allow => "allows",
        PolicyDecision::AskUser => "requires approval for",
        PolicyDecision::Deny => "denies",
    };
    PolicyEvaluation {
        decision,
        reason: format!(
            "agent profile {} ({}) {} {:?}",
            agent.name, agent.mode, action, request.risk
        ),
    }
}
