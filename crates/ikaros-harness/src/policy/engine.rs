// SPDX-License-Identifier: GPL-3.0-only

use super::{
    overlay::{evaluate_agent_policy_overlay, profile_workspace_write_decision},
    path::{has_component, is_protected, is_secret_like_path, is_under, resolve_under_workspace},
    rules::{is_destructive_command, is_forbidden_publication_or_git_action},
    types::{PolicyEngine, PolicyEvaluation, SandboxProfile},
};
use ikaros_core::{PolicyDecision, RiskLevel};
use ikaros_tools::PolicyRequest;

#[derive(Debug, Clone, Default)]
pub struct DefaultPolicyEngine;

impl PolicyEngine for DefaultPolicyEngine {
    fn evaluate(&self, request: &PolicyRequest, sandbox: &SandboxProfile) -> PolicyEvaluation {
        if is_forbidden_publication_or_git_action(&request.action)
            || request
                .command
                .as_deref()
                .is_some_and(is_forbidden_publication_or_git_action)
        {
            return deny(
                "git commit/push/tag or public publish actions require explicit release approval",
            );
        }

        if request
            .command
            .as_deref()
            .is_some_and(is_destructive_command)
            || matches!(request.risk, RiskLevel::Destructive)
        {
            return deny("destructive shell action is denied by default");
        }

        if matches!(request.risk, RiskLevel::SecretAccess) {
            return deny("secret access is denied by the default harness policy");
        }

        if matches!(request.risk, RiskLevel::SelfModify) {
            return deny("self-modification is disabled by default");
        }

        if let Some(path) = &request.path {
            let resolved = resolve_under_workspace(path, &sandbox.workspace_root);
            if is_protected(&resolved, sandbox)
                || has_component(path, ".temp")
                || has_component(&resolved, ".temp")
            {
                return deny("writes or direct tool access under .temp are denied");
            }
            if !is_under(&resolved, &sandbox.workspace_root) {
                if request.is_write {
                    return deny("workspace-external writes are denied");
                }
                return ask("workspace-external reads require approval");
            }
            if is_secret_like_path(&resolved) {
                if request.is_write
                    && matches!(
                        profile_workspace_write_decision(request, sandbox),
                        Some(PolicyDecision::Deny)
                    )
                {
                    return deny("agent profile denies workspace writes");
                }
                return ask("secret-looking path requires approval or a dedicated secret adapter");
            }
        }

        if let Some(evaluation) = evaluate_agent_policy_overlay(request, sandbox) {
            return evaluation;
        }

        if matches!(request.risk, RiskLevel::RemoteServer) {
            return allow(
                "remote-server action is allowed only for Ikaros-scoped deployment tests and must be logged",
            );
        }

        if matches!(request.risk, RiskLevel::Network) {
            return ask("network action requires an explicit provider or user approval");
        }

        if matches!(
            request.action.as_str(),
            "memory_append" | "memory_update" | "memory_delete" | "memory_candidate_create"
        ) && matches!(request.risk, RiskLevel::DatabaseWrite)
        {
            return allow("explicit local memory maintenance is allowed after secret detection");
        }

        if matches!(
            request.action.as_str(),
            "rag_ingest" | "rag_reindex" | "rag_delete_scope" | "rag_delete_path"
        ) && matches!(request.risk, RiskLevel::LocalWrite)
        {
            return allow("explicit local RAG maintenance writes only to the client-side index");
        }

        match request.risk {
            RiskLevel::SafeRead | RiskLevel::ShellRead => {
                allow("safe read action within harness scope")
            }
            RiskLevel::LocalWrite | RiskLevel::ShellWrite | RiskLevel::DatabaseWrite => {
                if request.is_write {
                    ask(
                        "write action requires approval unless explicitly granted by a goal-scoped policy",
                    )
                } else {
                    allow("non-write action accepted")
                }
            }
            RiskLevel::Network
            | RiskLevel::RemoteServer
            | RiskLevel::Destructive
            | RiskLevel::SecretAccess
            | RiskLevel::SelfModify => deny("risk level is not allowed by default"),
        }
    }
}

fn allow(reason: &str) -> PolicyEvaluation {
    PolicyEvaluation {
        decision: PolicyDecision::Allow,
        reason: reason.into(),
    }
}

fn ask(reason: &str) -> PolicyEvaluation {
    PolicyEvaluation {
        decision: PolicyDecision::AskUser,
        reason: reason.into(),
    }
}

fn deny(reason: &str) -> PolicyEvaluation {
    PolicyEvaluation {
        decision: PolicyDecision::Deny,
        reason: reason.into(),
    }
}
