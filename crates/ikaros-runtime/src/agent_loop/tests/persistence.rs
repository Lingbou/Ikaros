// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn recording_agent_runtime_captures_event_stream_for_replay() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let registry = SkillRegistry::new();
    let provider = SequenceProvider {
        calls: AtomicUsize::new(0),
        responses: vec![r#"{"final_answer":"recorded token=abc123"}"#.into()],
    };
    let runtime = RecordingAgentRuntime::new(HarnessAgentRuntime);

    let report = runtime
        .run_turn(
            AgentLoopInput {
                session_id: Some("recording-runtime-session".into()),
                turn_id: Some("recording-runtime-turn".into()),
                task_id: Some("recording-runtime-task".into()),
                system_prompt: "answer directly".into(),
                user_input: "hello token=abc123".into(),
            },
            &provider,
            &session,
            &registry,
            AgentLoopOptions::default(),
        )
        .await
        .expect("recorded turn");

    let recorded = runtime.recorded_events();
    assert_eq!(recorded, report.events);
    assert!(matches!(
        recorded.first().map(|event| &event.kind),
        Some(AgentEventKind::SessionStart)
    ));
    assert!(matches!(
        recorded.last().map(|event| &event.kind),
        Some(AgentEventKind::TurnEnd)
    ));
    let rendered = serde_json::to_string(&recorded).expect("events json");
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("abc123"));
}

#[tokio::test]
async fn agent_loop_can_persist_event_timeline_to_session_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let execution = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let session_store = Arc::new(SqliteSessionStore::new(temp.path().join("state")));
    let event_sink = PersistingAgentEventSink::new(session_store.clone()).with_agent_id("build");
    let mut registry = SkillRegistry::new();
    registry.register(EchoSkill {
        calls: Arc::new(AtomicUsize::new(0)),
    });
    let provider = NativeToolProvider {
        calls: AtomicUsize::new(0),
    };

    let report = super::super::run_agent_loop_with_events(
        AgentLoopInput {
            session_id: Some("persist-loop".into()),
            turn_id: None,
            task_id: Some("persist-loop".into()),
            system_prompt: "Use native tools when useful.".into(),
            user_input: "start token=abc123".into(),
        },
        &provider,
        &execution,
        &registry,
        &event_sink,
        AgentLoopOptions::default(),
    )
    .await
    .expect("agent loop");

    let replay = session_store
        .replay_session(&SessionId::from("persist-loop"))
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.session.agent_id.as_deref(), Some("build"));
    assert_eq!(replay.agent_events, report.events);
    assert!(matches!(
        replay.agent_events.first().map(|event| &event.kind),
        Some(AgentEventKind::SessionStart)
    ));
    let turn_start = replay
        .agent_events
        .iter()
        .find(|event| matches!(event.kind, AgentEventKind::TurnStart))
        .expect("turn start event");
    let prompt_sections = turn_start.payload["prompt"]["sections"]
        .as_array()
        .expect("prompt sections");
    assert!(
        prompt_sections.iter().any(|section| {
            section["kind"] == "tool_guidance" && section["source"] == "tooling"
        })
    );
    assert_eq!(
        turn_start.payload["prompt"]["section_count"],
        json!(prompt_sections.len())
    );
    let turn_start_json = serde_json::to_string(&turn_start.payload).expect("turn payload json");
    assert!(!turn_start_json.contains("Use native tools when useful."));
    assert!(!turn_start_json.contains("abc123"));
    assert!(matches!(
        replay.agent_events.last().map(|event| &event.kind),
        Some(AgentEventKind::TurnEnd)
    ));
    let tool_started = replay
        .agent_events
        .iter()
        .find(|event| {
            matches!(event.kind, AgentEventKind::ToolCallStarted)
                && event
                    .payload
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    == Some("loop_echo")
        })
        .expect("tool started replay event");
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallOutputDelta))
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallCompleted))
    );
    assert!(replay.agent_events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::AuditAnchor)
            && event
                .payload
                .get("tool_event_id")
                .and_then(serde_json::Value::as_str)
                == Some(tool_started.event_id.as_str())
            && event
                .payload
                .get("audit_kind")
                .and_then(serde_json::Value::as_str)
                == Some("tool_result")
    }));
    let replay_json = serde_json::to_string(&replay).expect("json");
    assert!(!replay_json.contains("abc123"));
    assert!(replay_json.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn agent_loop_persists_approval_records_to_session_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let execution = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let session_store = Arc::new(SqliteSessionStore::new(temp.path().join("state")));
    let event_sink = PersistingAgentTurnSink::new(session_store.clone()).with_agent_id("build");
    let mut registry = SkillRegistry::new();
    registry.register(WriteSkill);
    let provider = ApprovalToolProvider {
        calls: AtomicUsize::new(0),
    };

    let report = super::super::run_agent_loop_with_events(
        AgentLoopInput {
            session_id: Some("approval-loop".into()),
            turn_id: Some("approval-turn".into()),
            task_id: Some("approval-task".into()),
            system_prompt: "Use native tools when useful.".into(),
            user_input: "start".into(),
        },
        &provider,
        &execution,
        &registry,
        &event_sink,
        AgentLoopOptions::default(),
    )
    .await
    .expect("agent loop");
    event_sink.commit().expect("commit");

    assert_eq!(report.stop_reason, AgentLoopStopReason::WaitingForApproval);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ApprovalRequested))
    );
    assert!(report.events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::ApprovalRequested)
            && event
                .payload
                .get("tool_event_id")
                .and_then(serde_json::Value::as_str)
                .is_some()
            && event
                .payload
                .get("tool")
                .and_then(serde_json::Value::as_str)
                == Some("loop_write")
    }));
    let replay = session_store
        .replay_session(&SessionId::from("approval-loop"))
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.approvals.len(), 1);
    let approval_id = replay.approvals[0].approval_id.as_str();
    assert_eq!(
        replay.approvals[0].turn_id.as_ref().map(|id| id.as_str()),
        Some("approval-turn")
    );
    assert!(matches!(
        replay.approvals[0].status,
        ikaros_session::ApprovalStatus::Requested
    ));
    let approval_event = replay
        .agent_events
        .iter()
        .find(|event| matches!(event.kind, AgentEventKind::ApprovalRequested))
        .expect("approval requested event");
    assert_eq!(
        approval_event
            .payload
            .get("approval_id")
            .and_then(serde_json::Value::as_str),
        Some(approval_id)
    );
    let tool_event_id = approval_event
        .payload
        .get("tool_event_id")
        .and_then(serde_json::Value::as_str)
        .expect("tool event id");
    assert!(replay.agent_events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::AuditAnchor)
            && event
                .payload
                .get("tool_event_id")
                .and_then(serde_json::Value::as_str)
                == Some(tool_event_id)
            && event
                .payload
                .get("approval_id")
                .and_then(serde_json::Value::as_str)
                == Some(approval_id)
    }));
    let replay_json = serde_json::to_string(&replay).expect("json");
    assert!(!replay_json.contains("abc123"));
    assert!(replay_json.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn approval_resolution_updates_session_replay() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let execution = ExecutionSession::new(&workspace, &paths.audit_dir);
    let session_store = Arc::new(SqliteSessionStore::new(
        paths.home.join("agents").join("build"),
    ));
    let event_sink = PersistingAgentTurnSink::new(session_store.clone()).with_agent_id("build");
    let mut registry = SkillRegistry::new();
    registry.register(WriteSkill);
    let provider = ApprovalToolProvider {
        calls: AtomicUsize::new(0),
    };

    let report = super::super::run_agent_loop_with_events(
        AgentLoopInput {
            session_id: Some("approval-resolution-loop".into()),
            turn_id: Some("approval-resolution-turn".into()),
            task_id: Some("approval-resolution-task".into()),
            system_prompt: "Use native tools when useful.".into(),
            user_input: "start".into(),
        },
        &provider,
        &execution,
        &registry,
        &event_sink,
        AgentLoopOptions::default(),
    )
    .await
    .expect("agent loop");
    event_sink.commit().expect("commit");

    assert_eq!(report.stop_reason, AgentLoopStopReason::WaitingForApproval);
    let replay = session_store
        .replay_session(&SessionId::from("approval-resolution-loop"))
        .expect("replay")
        .expect("session exists");
    let approval_id = replay.approvals[0].approval_id.clone();
    let denied = execution
        .decide_approval(
            &approval_id,
            ikaros_harness::ApprovalStatus::Denied,
            Some("test denial token=abc123".into()),
        )
        .expect("deny");

    assert!(
        crate::record_approval_resolution(&paths, &workspace, Some("build"), &denied)
            .expect("record resolution")
    );
    let replay = session_store
        .replay_session(&SessionId::from("approval-resolution-loop"))
        .expect("replay")
        .expect("session exists");
    assert!(matches!(
        replay.approvals[0].status,
        ikaros_session::ApprovalStatus::Denied
    ));
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ApprovalResolved))
    );
    let replay_json = serde_json::to_string(&replay).expect("json");
    assert!(!replay_json.contains("abc123"));
    assert!(replay_json.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn agent_loop_provider_failure_persists_failed_turn() {
    let temp = tempfile::tempdir().expect("tempdir");
    let execution = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let session_store = Arc::new(SqliteSessionStore::new(temp.path().join("state")));
    let event_sink = PersistingAgentTurnSink::new(session_store.clone()).with_agent_id("build");
    let registry = SkillRegistry::new();

    let error = super::super::run_agent_loop_with_events(
        AgentLoopInput {
            session_id: Some("failed-loop".into()),
            turn_id: Some("failed-loop-turn".into()),
            task_id: Some("failed-loop-task".into()),
            system_prompt: "Fail clearly.".into(),
            user_input: "start token=abc123".into(),
        },
        &FailingProvider,
        &execution,
        &registry,
        &event_sink,
        AgentLoopOptions::default(),
    )
    .await
    .expect_err("provider should fail");
    assert!(error.to_string().contains("[REDACTED_SECRET]"));
    event_sink.commit().expect("commit failed turn");

    let replay = session_store
        .replay_session(&SessionId::from("failed-loop"))
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.session.agent_id.as_deref(), Some("build"));
    assert!(
        replay
            .agent_events
            .iter()
            .all(|event| event.turn_id.as_str() == "failed-loop-turn")
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::Error))
    );
    assert!(matches!(
        replay.agent_events.last().map(|event| &event.kind),
        Some(AgentEventKind::TurnEnd)
    ));
    assert_eq!(
        replay.agent_events.last().and_then(|event| {
            event
                .payload
                .get("status")
                .and_then(serde_json::Value::as_str)
        }),
        Some("failed")
    );
    let replay_json = serde_json::to_string(&replay).expect("json");
    assert!(!replay_json.contains("abc123"));
    assert!(replay_json.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn agent_loop_without_session_id_uses_fresh_session_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let registry = SkillRegistry::new();

    let first = run_agent_loop(
        AgentLoopInput {
            session_id: None,
            turn_id: None,
            task_id: None,
            system_prompt: "Answer directly.".into(),
            user_input: "first".into(),
        },
        &SequenceProvider {
            calls: AtomicUsize::new(0),
            responses: vec![r#"{"final_answer":"first done"}"#.into()],
        },
        &session,
        &registry,
        AgentLoopOptions::default(),
    )
    .await
    .expect("first loop");
    let second = run_agent_loop(
        AgentLoopInput {
            session_id: None,
            turn_id: None,
            task_id: None,
            system_prompt: "Answer directly.".into(),
            user_input: "second".into(),
        },
        &SequenceProvider {
            calls: AtomicUsize::new(0),
            responses: vec![r#"{"final_answer":"second done"}"#.into()],
        },
        &session,
        &registry,
        AgentLoopOptions::default(),
    )
    .await
    .expect("second loop");

    let first_session = &first.events[0].session_id;
    let second_session = &second.events[0].session_id;
    assert_ne!(first_session.as_str(), "local");
    assert_ne!(second_session.as_str(), "local");
    assert_ne!(first_session, second_session);
}
