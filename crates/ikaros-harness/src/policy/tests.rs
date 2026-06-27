// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::{
    AgentAuthScope, AgentInstance, AgentPermission, AgentProfile, PolicyDecision,
    ResolvedAgentProfile, RiskLevel,
};
use ikaros_toolkit::PolicyRequest;
#[cfg(unix)]
use std::{fs, os::unix::fs::symlink, path::PathBuf};

#[test]
fn denies_temp_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sandbox = SandboxProfile::new(temp.path());
    let request = PolicyRequest {
        action: "fs_write_guarded".into(),
        risk: RiskLevel::LocalWrite,
        path: Some(temp.path().join(".temp/file.txt")),
        command: None,
        is_write: true,
    };
    let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
    assert_eq!(decision.decision, PolicyDecision::Deny);
}

#[test]
fn denies_workspace_external_write() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sandbox = SandboxProfile::new(temp.path().join("workspace"));
    let request = PolicyRequest {
        action: "fs_write_guarded".into(),
        risk: RiskLevel::LocalWrite,
        path: Some(temp.path().join("outside.txt")),
        command: None,
        is_write: true,
    };
    let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
    assert_eq!(decision.decision, PolicyDecision::Deny);
}

#[test]
fn asks_for_workspace_external_read() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sandbox = SandboxProfile::new(temp.path().join("workspace"));
    let request = PolicyRequest {
        action: "fs_read".into(),
        risk: RiskLevel::SafeRead,
        path: Some(temp.path().join("outside.txt")),
        command: None,
        is_write: false,
    };
    let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
    assert_eq!(decision.decision, PolicyDecision::AskUser);
}

#[cfg(unix)]
#[test]
fn denies_workspace_external_write_through_symlink() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    symlink(&outside, workspace.join("outside_link")).expect("symlink");
    let sandbox = SandboxProfile::new(&workspace);
    let request = PolicyRequest {
        action: "fs_write_guarded".into(),
        risk: RiskLevel::LocalWrite,
        path: Some(PathBuf::from("outside_link/owned.txt")),
        command: None,
        is_write: true,
    };
    let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
    assert_eq!(decision.decision, PolicyDecision::Deny);
    assert!(decision.reason.contains("workspace-external writes"));
}

#[cfg(unix)]
#[test]
fn asks_for_workspace_external_read_through_symlink() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(outside.join("secret.txt"), "outside").expect("outside file");
    symlink(&outside, workspace.join("outside_link")).expect("symlink");
    let sandbox = SandboxProfile::new(&workspace);
    let request = PolicyRequest {
        action: "fs_read".into(),
        risk: RiskLevel::SafeRead,
        path: Some(PathBuf::from("outside_link/secret.txt")),
        command: None,
        is_write: false,
    };
    let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
    assert_eq!(decision.decision, PolicyDecision::AskUser);
    assert!(decision.reason.contains("workspace-external reads"));
}

#[cfg(unix)]
#[test]
fn denies_protected_temp_access_through_symlink() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join(".temp")).expect("temp");
    symlink(workspace.join(".temp"), workspace.join("temp_link")).expect("symlink");
    let sandbox = SandboxProfile::new(&workspace);
    let request = PolicyRequest {
        action: "fs_read".into(),
        risk: RiskLevel::SafeRead,
        path: Some(PathBuf::from("temp_link/secret.txt")),
        command: None,
        is_write: false,
    };
    let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
    assert_eq!(decision.decision, PolicyDecision::Deny);
    assert!(decision.reason.contains(".temp"));
}

#[test]
fn denies_destructive_command_and_git_commit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sandbox = SandboxProfile::new(temp.path());
    for command in ["rm -rf /tmp/thing", "git commit -m nope"] {
        let request = PolicyRequest {
            action: "shell_guarded".into(),
            risk: RiskLevel::ShellWrite,
            path: None,
            command: Some(command.into()),
            is_write: true,
        };
        let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
        assert_eq!(decision.decision, PolicyDecision::Deny);
    }
}

#[test]
fn allows_explicit_local_memory_maintenance() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sandbox = SandboxProfile::new(temp.path());
    for action in [
        "memory_append",
        "memory_update",
        "memory_delete",
        "memory_candidate_create",
    ] {
        let request = PolicyRequest {
            action: action.into(),
            risk: RiskLevel::DatabaseWrite,
            path: None,
            command: None,
            is_write: true,
        };
        let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
        assert_eq!(
            decision.decision,
            PolicyDecision::Allow,
            "{action} should be treated as explicit local memory maintenance"
        );
    }
}

#[test]
fn denies_self_modify_even_for_permissive_agent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let agent = ResolvedAgentProfile {
        name: "build".into(),
        profile: AgentProfile::build(),
    };
    let sandbox = SandboxProfile::new(temp.path()).with_agent(&agent);
    let request = PolicyRequest {
        action: "self_modify_apply".into(),
        risk: RiskLevel::SelfModify,
        path: Some(temp.path().join("crates/ikaros-runtime/src/lib.rs")),
        command: None,
        is_write: true,
    };
    let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
    assert_eq!(decision.decision, PolicyDecision::Deny);
    assert!(decision.reason.contains("self-modification"));
}

#[test]
fn plan_agent_denies_workspace_and_database_writes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let agent = ResolvedAgentProfile {
        name: "plan".into(),
        profile: AgentProfile::plan(),
    };
    let sandbox = SandboxProfile::new(temp.path()).with_agent(&agent);
    for (risk, action) in [
        (RiskLevel::LocalWrite, "fs_write_guarded"),
        (RiskLevel::DatabaseWrite, "memory_append"),
    ] {
        let request = PolicyRequest {
            action: action.into(),
            risk,
            path: Some(temp.path().join("note.txt")),
            command: None,
            is_write: true,
        };
        let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
        assert_eq!(decision.decision, PolicyDecision::Deny);
        assert!(decision.reason.contains("agent profile plan"));
    }
}

#[test]
fn build_agent_preserves_shell_read_behavior() {
    let temp = tempfile::tempdir().expect("tempdir");
    let agent = ResolvedAgentProfile {
        name: "build".into(),
        profile: AgentProfile::build(),
    };
    let sandbox = SandboxProfile::new(temp.path()).with_agent(&agent);
    let request = PolicyRequest {
        action: "git_diff".into(),
        risk: RiskLevel::ShellRead,
        path: None,
        command: Some("git diff".into()),
        is_write: false,
    };
    let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
    assert_eq!(decision.decision, PolicyDecision::Allow);
}

#[test]
fn agent_network_overlay_applies_before_network_defaults() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut profile = AgentProfile::general();
    profile.network = ikaros_core::AgentPermission::Deny;
    let agent = ResolvedAgentProfile {
        name: "offline".into(),
        profile,
    };
    let sandbox = SandboxProfile::new(temp.path()).with_agent(&agent);
    for risk in [RiskLevel::Network, RiskLevel::RemoteServer] {
        let request = PolicyRequest {
            action: "network_call".into(),
            risk,
            path: None,
            command: None,
            is_write: false,
        };
        let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
        assert_eq!(decision.decision, PolicyDecision::Deny);
    }
}

#[test]
fn agent_instance_auth_scope_tightens_network_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut profile = AgentProfile::general();
    profile.network = AgentPermission::Allow;
    let resolved = ResolvedAgentProfile {
        name: "online-profile".into(),
        profile,
    };
    let mut instance = AgentInstance::local(resolved, temp.path(), temp.path());
    instance.agent_id = "offline-instance".into();
    instance.auth_scope = AgentAuthScope {
        local_only: true,
        allow_network: AgentPermission::Deny,
    };
    let sandbox = SandboxProfile::new(temp.path()).with_agent_instance(&instance);
    let request = PolicyRequest {
        action: "cloud_call".into(),
        risk: RiskLevel::Network,
        path: None,
        command: None,
        is_write: false,
    };

    let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);

    assert_eq!(decision.decision, PolicyDecision::Deny);
    assert!(decision.reason.contains("offline-instance"));
}

#[test]
fn network_write_combines_agent_network_and_workspace_write_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let agent = ResolvedAgentProfile {
        name: "plan".into(),
        profile: AgentProfile::plan(),
    };
    let sandbox = SandboxProfile::new(temp.path()).with_agent(&agent);
    let request = PolicyRequest {
        action: "rag_ingest".into(),
        risk: RiskLevel::Network,
        path: Some(temp.path().join("doc.md")),
        command: None,
        is_write: true,
    };
    let decision = DefaultPolicyEngine.evaluate(&request, &sandbox);
    assert_eq!(decision.decision, PolicyDecision::Deny);
    assert!(decision.reason.contains("agent profile plan"));
}
