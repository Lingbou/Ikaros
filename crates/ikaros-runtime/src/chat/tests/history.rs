// SPDX-License-Identifier: GPL-3.0-only

use super::*;

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
