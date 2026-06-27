// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn stale_legacy_chat_history_path_does_not_affect_session_backed_turn() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    fs::create_dir_all(paths.home.join("chat/history.jsonl")).expect("history dir");

    let result = run_chat_message(
        "store failure token=abc123",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            agent_loop: false,
            no_context: true,
            relationship_learning: false,
            session_id: Some("history-failure-session".into()),
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("session-backed chat should ignore stale legacy history path");
    assert_eq!(result.chat_session_id, "history-failure-session");

    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let replay = session_store
        .replay_session(&SessionId::from("history-failure-session"))
        .expect("replay")
        .expect("failed session exists");
    assert_eq!(replay.entries.len(), 2);
    assert_eq!(replay.entries[0].kind, SessionEntryKind::UserMessage);
    assert_eq!(replay.entries[1].kind, SessionEntryKind::AssistantMessage);
    assert!(replay.agent_events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::TurnEnd)
            && event
                .payload
                .get("status")
                .and_then(serde_json::Value::as_str)
                == Some("completed")
    }));
    assert!(!replay.agent_events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::Error)
            && event
                .payload
                .get("phase")
                .and_then(serde_json::Value::as_str)
                == Some("chat_history_append")
    }));
    let replay_json = serde_json::to_string(&replay).expect("replay json");
    assert!(!replay_json.contains("abc123"));
    assert!(replay_json.contains("[REDACTED_SECRET]"));

    let projected = super::super::history::chat_history_records_from_session_replay(&replay);
    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].session_id, "history-failure-session");
    assert_eq!(
        projected[0].turn_id,
        replay.entries[1]
            .turn_id
            .as_ref()
            .expect("assistant entry turn id")
            .as_str()
    );
    assert_eq!(projected[0].agent, "build");
    assert_eq!(projected[0].provider, "mock");
    assert_eq!(projected[0].model, "mock-ikaros");
    assert!(!projected[0].user_message.contains("abc123"));
    assert!(!projected[0].assistant_message.contains("abc123"));
    assert!(projected[0].user_message.contains("[REDACTED_SECRET]"));
}

#[test]
fn chat_history_searches_session_replay_projection_records() {
    let records = vec![
        chat_history_record("session-a", "alpha first"),
        chat_history_record("session-b", "beta second"),
        chat_history_record("session-a", "alpha follow-up token=abc123"),
    ];

    let matches =
        super::super::history::search_chat_history_records(records.clone(), "alpha", 10, None);
    assert_eq!(matches.len(), 2);
    assert_eq!(matches[0].session_id, "session-a");
    assert!(matches[0].user_message.contains("follow-up"));
    assert!(!matches[0].user_message.contains("abc123"));
    assert!(matches[0].user_message.contains("[REDACTED_SECRET]"));

    let session_b = super::super::history::search_chat_history_records(
        records.clone(),
        "alpha",
        10,
        Some("session-b"),
    );
    assert!(session_b.is_empty());

    let redacted = super::super::history::search_chat_history_records(
        records.clone(),
        "token=abc123",
        10,
        Some("session-a"),
    );
    assert_eq!(redacted.len(), 1);
    assert!(redacted[0].user_message.contains("[REDACTED_SECRET]"));

    assert!(
        super::super::history::search_chat_history_records(records, "alpha", 0, None).is_empty()
    );
}
