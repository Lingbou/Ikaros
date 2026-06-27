// SPDX-License-Identifier: GPL-3.0-only

use super::{AgentPoolTask, run_agent_handoff, run_agent_handoff_with_options, run_agent_pool};
use crate::task::TaskRunOptions;
use ikaros_core::{IkarosPaths, TaskState};
use ikaros_session::{AgentEventKind, SessionId, SessionSource, SessionStore, SqliteSessionStore};

#[tokio::test]
async fn agent_handoff_records_audit_event() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let report = run_agent_handoff(&paths, &workspace, Some("build"), "inspect runtime", true)
        .await
        .expect("handoff");

    assert_eq!(report.agent, "build");
    assert!(report.dry_run);
    let audit = std::fs::read_to_string(report.audit_path).expect("audit");
    assert!(audit.contains("\"kind\":\"agent_handoff\""));
    assert!(audit.contains("\"dry_run\":true"));
}

#[tokio::test]
async fn agent_pool_runs_multiple_dry_run_handoffs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let report = run_agent_pool(
        &paths,
        &workspace,
        vec![
            AgentPoolTask::new("inspect runtime", None),
            AgentPoolTask::new("inspect harness", Some("plan".into())),
        ],
        Some("build"),
        true,
        2,
    )
    .await
    .expect("pool");

    assert_eq!(report.total, 2);
    assert_eq!(report.succeeded, 2);
    assert_eq!(report.failed, 0);
    assert_eq!(report.concurrency, 2);
    assert_eq!(report.reports[0].profile.as_deref(), Some("build"));
    assert_eq!(report.reports[1].profile.as_deref(), Some("plan"));
    assert!(report.reports.iter().all(|item| item.ok));
    let audit = std::fs::read_to_string(paths.audit_dir.join("audit.jsonl")).expect("audit");
    assert!(audit.matches("\"kind\":\"agent_handoff\"").count() >= 2);
}

#[tokio::test]
async fn agent_pool_redacts_secret_like_task_text_in_reports() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);

    let report = run_agent_pool(
        &paths,
        &workspace,
        vec![AgentPoolTask::new("inspect token=abc123", None)],
        Some("missing-profile"),
        true,
        1,
    )
    .await
    .expect("pool report");
    let rendered = serde_json::to_string(&report).expect("json");

    assert_eq!(report.failed, 1);
    assert!(!rendered.contains("abc123"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn agent_handoff_can_use_agent_loop() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let report = run_agent_handoff_with_options(
        &paths,
        &workspace,
        Some("build"),
        "inspect runtime",
        TaskRunOptions::agent_loop(true),
    )
    .await
    .expect("agent loop handoff");

    assert_eq!(report.agent, "build");
    assert!(report.dry_run);
    assert!(report.agent_loop);
    assert!(report.loop_report.is_some());
    assert_eq!(report.report.state, TaskState::Completed);
    let audit = std::fs::read_to_string(report.audit_path).expect("audit");
    assert!(audit.contains("\"kind\":\"agent_loop_start\""));
    assert!(audit.contains("\"kind\":\"agent_handoff\""));
    assert!(audit.contains("\"agent_loop\":true"));

    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let replay = session_store
        .replay_session(&SessionId::from(report.task_id.as_str()))
        .expect("replay")
        .expect("handoff session");
    assert!(matches!(
        replay.session.source,
        SessionSource::Subagent { .. }
    ));
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::TurnStart))
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::TurnEnd))
    );
    assert!(replay.agent_events.iter().any(|event| matches!(
        event.kind,
        AgentEventKind::UserMessage
    ) && event.payload["content"].as_str()
        == Some("inspect runtime")));
}

#[tokio::test]
async fn agent_handoff_rejects_excessive_delegation_depth() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let mut options = TaskRunOptions::deterministic(true);
    options.delegation_depth = 3;
    let error = run_agent_handoff_with_options(
        &paths,
        &workspace,
        Some("build"),
        "inspect runtime",
        options,
    )
    .await
    .expect_err("delegation depth over policy limit must fail");

    assert!(error.to_string().contains("delegation depth"));
}

fn write_offline_mock_config(paths: &IkarosPaths) {
    std::fs::create_dir_all(&paths.home).expect("home");
    std::fs::write(
        &paths.config,
        r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
"#,
    )
    .expect("mock config");
}
