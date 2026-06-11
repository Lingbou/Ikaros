// SPDX-License-Identifier: GPL-3.0-only

use super::initial_interactive_runtime;
use super::interactive::{
    available_agent_lines, format_interactive_chat_status, resolve_interactive_agent,
};
use ikaros_core::{
    AgentAuthScope, AgentInstanceConfig, AgentPermission, IkarosConfig, PolicyDecision,
};
use ikaros_harness::ExecutionSession;
use ikaros_models::ModelUsageLedger;
use ikaros_runtime::{ChatHistoryStore, ChatRunOptions};
use std::fs;

#[test]
fn interactive_chat_lists_and_resolves_agent_profiles() {
    let config = IkarosConfig::default();
    let lines = available_agent_lines(&config, "plan");
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("* plan mode=plan"))
    );
    assert!(lines.iter().any(|line| line.contains("build mode=build")));

    let agent = resolve_interactive_agent(&config, "general").expect("general");
    assert_eq!(agent.name, "general");
    assert_eq!(agent.mode().as_str(), "general");

    let error = resolve_interactive_agent(&config, "missing").expect_err("missing");
    assert!(error.to_string().contains("agent profile not found"));
}

#[test]
fn interactive_chat_status_reports_active_runtime() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let paths = ikaros_core::IkarosPaths::from_home(temp.path().join("home"));
    paths.ensure().expect("paths");
    let agent = IkarosConfig::default().agent.active();
    let session = ExecutionSession::new_with_agent(&workspace, &paths.audit_dir, &agent);
    let usage = ModelUsageLedger::new(&paths.audit_dir);
    let history = ChatHistoryStore::new(&paths.home);
    let options = ChatRunOptions {
        stream: true,
        scope: Some("repo".into()),
        ..ChatRunOptions::default()
    };

    let status = format_interactive_chat_status(
        &agent,
        &session,
        "chat-session",
        &options,
        "Neutral",
        &usage,
        &history,
    );
    assert!(status.contains("agent=build"));
    assert!(status.contains("mode=build"));
    assert!(status.contains("emotion=Neutral"));
    assert!(status.contains("stream=true"));
    assert!(status.contains("history_context_limit=3"));
    assert!(status.contains("history_summary_limit=12"));
    assert!(status.contains("context_char_budget=8000"));
    assert!(status.contains("relationship_learning=true"));
    assert!(status.contains("agent_loop=true"));
    assert!(status.contains("scope=repo"));
    assert!(status.contains("chat_session=chat-session"));
    assert!(status.contains("audit="));
    assert!(status.contains("model_usage="));
    assert!(status.contains("chat_history="));
}

#[test]
fn interactive_chat_initial_runtime_resolves_agent_instances() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = ikaros_core::IkarosPaths::from_home(temp.path().join("home"));
    paths.ensure().expect("paths");
    let fallback_workspace = temp.path().join("fallback-workspace");
    let instance_workspace = temp.path().join("instance-workspace");
    fs::create_dir_all(&fallback_workspace).expect("fallback workspace");
    fs::create_dir_all(&instance_workspace).expect("instance workspace");
    let mut config = IkarosConfig::default();
    config.agent.instances.insert(
        "repo-build".into(),
        AgentInstanceConfig {
            profile: "build".into(),
            workspace: Some(instance_workspace.clone()),
            auth_scope: AgentAuthScope {
                local_only: true,
                allow_network: AgentPermission::Deny,
            },
            ..AgentInstanceConfig::default()
        },
    );

    let (runtime, _registry) = initial_interactive_runtime(
        &paths,
        &fallback_workspace,
        &config,
        Some("repo-build"),
        "chat-session".into(),
    )
    .expect("interactive runtime");

    assert_eq!(runtime.agent.name, "build");
    assert_eq!(runtime.session.sandbox.workspace_root, instance_workspace);
    let overlay = runtime
        .session
        .sandbox
        .agent
        .as_ref()
        .expect("agent overlay");
    assert_eq!(overlay.agent_id.as_deref(), Some("repo-build"));
    assert_eq!(overlay.network, PolicyDecision::Deny);
}
