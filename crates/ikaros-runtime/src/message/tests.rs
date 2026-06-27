// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::IkarosPaths;
use ikaros_gateway::{
    GatewayMessage, GatewayMessageKind, GatewayMessageStatus, GatewayRoute, GatewaySessionSource,
    LocalGatewayStore,
};
use ikaros_session::{
    AgentEventKind, SessionInputStatus, SessionSource, SessionStore, SqliteSessionStore,
};

#[test]
fn gateway_session_id_uses_digest_without_raw_route_identity() {
    let first_source = GatewaySessionSource {
        channel: "telegram".into(),
        account: Some("account-a".into()),
        peer: Some("peer-a".into()),
        thread: Some("thread-a".into()),
        message_id: Some("message-1".into()),
    };
    let second_source = GatewaySessionSource {
        message_id: Some("message-2".into()),
        ..first_source.clone()
    };
    let first = GatewayMessage::new(
        GatewayRoute::new(
            "telegram",
            GatewayMessageKind::Chat,
            "hello",
            Some("build".into()),
        )
        .with_session_source(first_source),
    )
    .expect("first");
    let second = GatewayMessage::new(
        GatewayRoute::new(
            "telegram",
            GatewayMessageKind::Chat,
            "continue",
            Some("build".into()),
        )
        .with_session_source(second_source),
    )
    .expect("second");

    let first_id = crate::session::gateway_session_id(&first);
    let second_id = crate::session::gateway_session_id(&second);

    assert_eq!(first_id, second_id);
    assert!(first_id.as_str().starts_with("gateway-"));
    assert!(!first_id.as_str().contains("telegram"));
    assert!(!first_id.as_str().contains("account-a"));
    assert!(!first_id.as_str().contains("peer-a"));
    assert!(!first_id.as_str().contains("thread-a"));
    assert!(!first_id.as_str().contains("message-1"));

    let source = crate::session::gateway_session_source(&first);
    match source {
        SessionSource::Gateway { message_id, .. } => {
            assert_eq!(message_id.as_deref(), Some("message-1"));
        }
        other => panic!("unexpected source: {other:?}"),
    }
}

#[test]
fn gateway_session_id_changes_for_distinct_gateway_threads() {
    let base = GatewaySessionSource {
        channel: "telegram".into(),
        account: Some("account-a".into()),
        peer: Some("peer-a".into()),
        thread: Some("thread-a".into()),
        message_id: Some("message-1".into()),
    };
    let first = GatewayMessage::new(
        GatewayRoute::new("telegram", GatewayMessageKind::Chat, "hello", None)
            .with_session_source(base.clone()),
    )
    .expect("first");
    let second = GatewayMessage::new(
        GatewayRoute::new("telegram", GatewayMessageKind::Chat, "hello", None).with_session_source(
            GatewaySessionSource {
                thread: Some("thread-b".into()),
                ..base
            },
        ),
    )
    .expect("second");

    assert_ne!(
        crate::session::gateway_session_id(&first),
        crate::session::gateway_session_id(&second)
    );
}

#[test]
fn gateway_safe_tools_restricts_agent_loop_toolsets_to_core() {
    let agent = ikaros_core::IkarosConfig::default().agent.active();
    let mut safe_message = GatewayMessage::new(GatewayRoute::new(
        "telegram",
        GatewayMessageKind::Task,
        "run task",
        Some("build".into()),
    ))
    .expect("safe message");
    safe_message.safe_tools = true;
    let safe_toolsets =
        super::drain::gateway_toolsets_for_message(&agent, &safe_message).expect("safe toolsets");
    assert_eq!(safe_toolsets.names(), vec!["core"]);

    let regular_message = GatewayMessage::new(GatewayRoute::new(
        "telegram",
        GatewayMessageKind::Task,
        "run task",
        Some("build".into()),
    ))
    .expect("regular message");
    let regular_toolsets = super::drain::gateway_toolsets_for_message(&agent, &regular_message)
        .expect("regular toolsets");
    assert!(regular_toolsets.names().contains(&"workspace"));
    assert!(regular_toolsets.names().contains(&"coding"));
}

#[tokio::test]
async fn drain_gateway_chat_messages_with_same_thread_resume_same_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let source = GatewaySessionSource {
        channel: "telegram".into(),
        account: Some("account-a".into()),
        peer: Some("peer-a".into()),
        thread: Some("thread-a".into()),
        message_id: Some("message-1".into()),
    };
    let first = store
        .enqueue(
            GatewayRoute::new(
                "telegram",
                GatewayMessageKind::Chat,
                "hello first",
                Some("build".into()),
            )
            .with_session_source(source.clone()),
        )
        .expect("first");
    let second = store
        .enqueue(
            GatewayRoute::new(
                "telegram",
                GatewayMessageKind::Chat,
                "hello second",
                Some("build".into()),
            )
            .with_session_source(GatewaySessionSource {
                message_id: Some("message-2".into()),
                ..source
            }),
        )
        .expect("second");

    let reports = drain_gateway_messages(
        store
            .claim_pending_with_owner(10, "runtime-worker")
            .expect("claim"),
        &store,
        &paths,
        &workspace,
        None,
    )
    .await
    .expect("drain");

    assert_eq!(reports.len(), 2);
    let session_id = crate::session::gateway_session_id(&first);
    assert_eq!(session_id, crate::session::gateway_session_id(&second));
    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let replay = session_store
        .replay_session(&session_id)
        .expect("replay")
        .expect("gateway chat session");
    assert!(
        replay
            .entries
            .iter()
            .any(|entry| entry.visible_text.as_deref() == Some("hello first"))
    );
    assert!(
        replay
            .entries
            .iter()
            .any(|entry| entry.visible_text.as_deref() == Some("hello second"))
    );
    assert!(matches!(
        replay.session.source,
        SessionSource::Gateway { .. }
    ));
}

#[tokio::test]
async fn drain_gateway_chat_message_records_delivery_and_redacts_inbox() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let queued = store
        .enqueue(GatewayRoute::new(
            "test",
            GatewayMessageKind::Chat,
            "hello token=abc123",
            Some("build".into()),
        ))
        .expect("enqueue");

    let reports = drain_gateway_messages(
        store
            .claim_pending_with_owner(10, "runtime-worker")
            .expect("claim"),
        &store,
        &paths,
        &workspace,
        None,
    )
    .await
    .expect("drain");

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].message_id, queued.id);
    assert_eq!(reports[0].kind, "chat");
    assert_eq!(reports[0].status, GatewayMessageStatus::Processed);
    assert_eq!(reports[0].provider.as_deref(), Some("mock"));
    assert_eq!(store.deliveries().expect("deliveries").len(), 1);
    let inbox = std::fs::read_to_string(store.inbox_path()).expect("inbox");
    assert!(!inbox.contains("abc123"));
    assert!(inbox.contains("[REDACTED_SECRET]"));

    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let session_id = crate::session::gateway_session_id(&queued);
    let replay = session_store
        .replay_session(&session_id)
        .expect("replay")
        .expect("gateway chat session");
    assert!(matches!(
        replay.session.source,
        SessionSource::Gateway { .. }
    ));
    assert!(replay.entries.len() >= 3);
    assert!(
        replay
            .entries
            .iter()
            .any(|entry| entry.payload["kind"] == "gateway_delivery")
    );
    let inputs = session_store
        .session_inputs(&session_id)
        .expect("gateway chat inputs");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].status, SessionInputStatus::Promoted);
    assert_eq!(
        inputs[0].promoted_turn_id.as_ref(),
        Some(&crate::session::gateway_turn_id(&queued.id))
    );
    assert_eq!(
        inputs[0].payload["content"].as_str(),
        Some("hello token=[REDACTED_SECRET]")
    );
}

#[tokio::test]
async fn drain_gateway_task_message_records_task_report_delivery() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let queued = store
        .enqueue(GatewayRoute::new(
            "test",
            GatewayMessageKind::Task,
            "summarize runtime gateway",
            Some("build".into()),
        ))
        .expect("enqueue");

    let reports = drain_gateway_messages(
        store.pending(10).expect("pending"),
        &store,
        &paths,
        &workspace,
        None,
    )
    .await
    .expect("drain");

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].message_id, queued.id);
    assert_eq!(reports[0].kind, "task");
    assert_eq!(reports[0].status, GatewayMessageStatus::Processed);
    assert!(reports[0].task_report.is_some());
    let deliveries = store.deliveries().expect("deliveries");
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].kind, "task_report");
    assert!(deliveries[0].content.contains("\"task_id\""));

    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let session_id = crate::session::gateway_session_id(&queued);
    let replay = session_store
        .replay_session(&session_id)
        .expect("replay")
        .expect("gateway task session");
    assert!(matches!(
        replay.session.source,
        SessionSource::Gateway { .. }
    ));
    assert_eq!(replay.entries.len(), 2);
    assert_eq!(
        replay.entries[0].visible_text.as_deref(),
        Some("summarize runtime gateway")
    );
    assert_eq!(replay.entries[1].payload["source"], "gateway");
    assert_eq!(replay.entries[1].payload["status"], "processed");
    assert!(replay.agent_events.len() >= 4);
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
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ModelStream(_)))
    );
    let inputs = session_store
        .session_inputs(&session_id)
        .expect("gateway task inputs");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].status, SessionInputStatus::Promoted);
    assert_eq!(
        inputs[0].promoted_turn_id.as_ref(),
        Some(&crate::session::gateway_turn_id(&queued.id))
    );
    assert_eq!(inputs[0].payload["kind"].as_str(), Some("task"));
    assert_eq!(
        inputs[0].payload["content"].as_str(),
        Some("summarize runtime gateway")
    );
}

#[tokio::test]
async fn drain_gateway_error_retries_then_dead_letters_message() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_unreachable_openai_config(&paths);
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let queued = store
        .enqueue(GatewayRoute::new(
            "test",
            GatewayMessageKind::Chat,
            "hello token=abc123",
            Some("build".into()),
        ))
        .expect("enqueue");

    let first_claim = store
        .claim_pending_with_owner(1, "runtime-worker")
        .expect("first claim");
    let first = drain_gateway_messages(first_claim, &store, &paths, &workspace, None)
        .await
        .expect("first drain");
    assert_eq!(first[0].message_id, queued.id);
    assert_eq!(first[0].status, GatewayMessageStatus::Pending);
    let after_first = store
        .list()
        .expect("list")
        .into_iter()
        .find(|message| message.id == queued.id)
        .expect("message");
    assert_eq!(after_first.status, GatewayMessageStatus::Pending);
    assert_eq!(after_first.attempt_count, 1);
    assert!(after_first.last_error.is_some());
    assert!(
        !after_first
            .last_error
            .expect("first error")
            .contains("abc123")
    );

    let second_claim = store
        .claim_pending_with_owner(1, "runtime-worker")
        .expect("second claim");
    let second = drain_gateway_messages(second_claim, &store, &paths, &workspace, None)
        .await
        .expect("second drain");
    assert_eq!(second[0].status, GatewayMessageStatus::DeadLettered);
    let after_second = store
        .list()
        .expect("list")
        .into_iter()
        .find(|message| message.id == queued.id)
        .expect("message");
    assert_eq!(after_second.status, GatewayMessageStatus::DeadLettered);
    assert_eq!(after_second.attempt_count, 2);
    assert!(after_second.dead_lettered_at.is_some());
}

#[tokio::test]
async fn drain_gateway_chat_preflight_error_records_typed_session_timeline() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::create_dir_all(&home).expect("home");
    let paths = IkarosPaths::from_home(home);
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let queued = store
        .enqueue(GatewayRoute::new(
            "test",
            GatewayMessageKind::Chat,
            "hello token=abc123",
            Some("build".into()),
        ))
        .expect("enqueue");

    let reports = drain_gateway_messages(
        store
            .claim_pending_with_owner(10, "runtime-worker")
            .expect("claim"),
        &store,
        &paths,
        &workspace,
        None,
    )
    .await
    .expect("drain");

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].message_id, queued.id);
    assert_eq!(reports[0].status, GatewayMessageStatus::Pending);
    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let session_id = crate::session::gateway_session_id(&queued);
    let replay = session_store
        .replay_session(&session_id)
        .expect("replay")
        .expect("gateway chat session");
    assert!(
        replay
            .entries
            .iter()
            .any(|entry| entry.visible_text.as_deref() == Some("hello token=[REDACTED_SECRET]"))
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::UserMessage))
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::Error)
                && !event.payload.to_string().contains("abc123"))
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::TurnEnd)
                && event.payload["status"] == "pending")
    );
}

#[tokio::test]
async fn gateway_worker_tick_drains_limited_pending_messages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let queued = store
        .enqueue(GatewayRoute::new(
            "test",
            GatewayMessageKind::Task,
            "summarize gateway worker",
            Some("build".into()),
        ))
        .expect("enqueue");

    let tick = run_gateway_worker_tick(&store, 1, &paths, &workspace, None)
        .await
        .expect("worker tick");

    assert_eq!(tick.kind, "gateway_worker_tick");
    assert_eq!(tick.pending, 1);
    assert_eq!(tick.drained, 1);
    assert_eq!(tick.reports[0].message_id, queued.id);
    assert_eq!(store.pending(10).expect("pending").len(), 0);
    assert_eq!(store.deliveries().expect("deliveries").len(), 1);
}

#[tokio::test]
async fn drain_gateway_claim_does_not_deliver_after_message_cancel() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let store = LocalGatewayStore::new(&paths.gateway_dir);
    let queued = store
        .enqueue(GatewayRoute::new(
            "test",
            GatewayMessageKind::Task,
            "cancel after claim",
            Some("build".into()),
        ))
        .expect("enqueue");
    let claimed = store.claim_pending(1).expect("claim");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, queued.id);

    let cancelled = store
        .cancel(&queued.id, "operator requested cancel")
        .expect("cancel")
        .expect("cancelled");
    assert_eq!(cancelled.status, GatewayMessageStatus::Cancelled);

    let reports = drain_gateway_messages(claimed, &store, &paths, &workspace, None)
        .await
        .expect("drain");

    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].message_id, queued.id);
    assert_eq!(reports[0].status, GatewayMessageStatus::Cancelled);
    assert!(reports[0].delivery.is_none());
    assert!(store.deliveries().expect("deliveries").is_empty());
    let listed = store.list().expect("list");
    assert_eq!(listed[0].status, GatewayMessageStatus::Cancelled);
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

fn write_unreachable_openai_config(paths: &IkarosPaths) {
    std::fs::create_dir_all(&paths.home).expect("home");
    std::fs::write(
        &paths.config,
        r#"schema_version: 1

providers:
  model:
    api_key: test-key
    base_url: http://127.0.0.1:9/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: unreachable-model
    timeout_ms: 1
    max_retries: 0
    rate_limit_per_minute: null
    daily_token_budget: null

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
    .expect("unreachable config");
}
