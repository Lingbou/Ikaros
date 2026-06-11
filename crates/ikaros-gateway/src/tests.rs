// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::now_rfc3339;
use std::{
    collections::BTreeSet,
    fs,
    sync::{Arc, Barrier},
    thread,
};

#[test]
fn gateway_enqueues_and_lists_pending_messages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());

    let chat = store
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Chat,
            "hello",
            None,
        ))
        .expect("chat");
    let task = store
        .enqueue(GatewayRoute::new(
            "webhook",
            GatewayMessageKind::Task,
            "summarize project",
            Some("plan".into()),
        ))
        .expect("task");

    let listed = store.list().expect("list");
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].id, chat.id);
    assert_eq!(listed[1].id, task.id);

    let pending = store.pending(1).expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, chat.id);
}

#[test]
fn gateway_enqueues_protocol_routes_with_idempotency() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let source = GatewaySessionSource {
        channel: "slack".into(),
        account: Some("team".into()),
        peer: Some("user".into()),
        thread: Some("thread".into()),
        message_id: Some("message".into()),
    };
    let route = GatewayRoute::from_protocol_request(
        source,
        GatewayRequest::task("summarize runtime").with_agent("plan"),
    )
    .with_idempotency_key("slack:thread:message")
    .with_client(
        GatewayClientIdentity::new("desktop"),
        vec![GatewayCapability::new("streaming")],
    );

    let first = store.enqueue(route.clone()).expect("first");
    let second = store.enqueue(route).expect("second");
    let listed = store.list().expect("list");

    assert_eq!(first.id, second.id);
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0]
            .session_source
            .as_ref()
            .expect("session source")
            .channel,
        "slack"
    );
    assert_eq!(
        listed[0].idempotency_key.as_deref(),
        Some("slack:thread:message")
    );
    assert_eq!(
        listed[0]
            .client_identity
            .as_ref()
            .expect("client identity")
            .client_id,
        "desktop"
    );
    assert_eq!(listed[0].capabilities[0].name, "streaming");
}

#[test]
fn gateway_idempotency_uses_digest_not_redacted_display_value() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());

    let first = store
        .enqueue(
            GatewayRoute::new("webhook", GatewayMessageKind::Task, "first", None)
                .with_idempotency_key("token=a"),
        )
        .expect("first");
    let duplicate = store
        .enqueue(
            GatewayRoute::new("webhook", GatewayMessageKind::Task, "duplicate", None)
                .with_idempotency_key("token=a"),
        )
        .expect("duplicate");
    let second = store
        .enqueue(
            GatewayRoute::new("webhook", GatewayMessageKind::Task, "second", None)
                .with_idempotency_key("token=b"),
        )
        .expect("second");

    let listed = store.list().expect("list");
    assert_eq!(first.id, duplicate.id);
    assert_ne!(first.id, second.id);
    assert_eq!(listed.len(), 2);
    assert_eq!(
        listed[0].idempotency_key.as_deref(),
        Some("token=[REDACTED_SECRET]")
    );
    assert_ne!(
        listed[0].idempotency_key_digest.as_deref(),
        listed[1].idempotency_key_digest.as_deref()
    );

    let raw = fs::read_to_string(store.inbox_path()).expect("inbox");
    assert!(!raw.contains("token=a"));
    assert!(!raw.contains("token=b"));
    assert!(raw.contains("sha256:"));
}

#[test]
fn gateway_records_status_and_deletes_messages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let message = store
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Task,
            "run local task",
            None,
        ))
        .expect("message");

    let updated = store
        .record_status(&message.id, GatewayMessageStatus::Processed, "ok")
        .expect("record")
        .expect("found");
    assert_eq!(updated.status, GatewayMessageStatus::Processed);
    assert_eq!(updated.summary.as_deref(), Some("ok"));
    assert!(updated.processed_at.is_some());
    assert!(store.pending(10).expect("pending").is_empty());

    assert!(store.delete(&message.id).expect("delete"));
    assert!(store.list().expect("list").is_empty());
}

#[test]
fn gateway_claims_pending_messages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let first = store
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Task,
            "first",
            None,
        ))
        .expect("first");
    let second = store
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Task,
            "second",
            None,
        ))
        .expect("second");

    let claimed = store.claim_pending(1).expect("claim");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, first.id);
    assert_eq!(claimed[0].status, GatewayMessageStatus::Processing);

    let pending = store.pending(10).expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].id, second.id);
}

#[test]
fn gateway_reclaims_stale_processing_messages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let mut stale = GatewayMessage::new(GatewayRoute::new(
        "cli",
        GatewayMessageKind::Task,
        "stale",
        None,
    ))
    .expect("stale");
    stale.status = GatewayMessageStatus::Processing;
    stale.updated_at = "2020-01-01T00:00:00Z".into();

    let mut fresh = GatewayMessage::new(GatewayRoute::new(
        "cli",
        GatewayMessageKind::Task,
        "fresh",
        None,
    ))
    .expect("fresh");
    fresh.status = GatewayMessageStatus::Processing;
    fresh.updated_at = now_rfc3339().expect("now");

    fs::create_dir_all(temp.path()).expect("gateway dir");
    fs::write(
        store.inbox_path(),
        format!(
            "{}\n{}\n",
            serde_json::to_string(&stale).expect("stale json"),
            serde_json::to_string(&fresh).expect("fresh json")
        ),
    )
    .expect("write inbox");

    let claimed = store.claim_pending(10).expect("claim");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, stale.id);

    let listed = store.list().expect("list");
    let fresh = listed
        .iter()
        .find(|message| message.id == fresh.id)
        .expect("fresh listed");
    assert_eq!(fresh.status, GatewayMessageStatus::Processing);
}

#[test]
fn concurrent_gateway_enqueues_preserve_all_messages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(LocalGatewayStore::new(temp.path()));
    let workers = 8usize;
    let per_worker = 25usize;
    let barrier = Arc::new(Barrier::new(workers));
    let handles = (0..workers)
        .map(|worker| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                for index in 0..per_worker {
                    store
                        .enqueue(GatewayRoute::new(
                            format!("worker-{worker}"),
                            GatewayMessageKind::Task,
                            format!("message-{worker}-{index}"),
                            None,
                        ))
                        .expect("enqueue");
                }
            })
        })
        .collect::<Vec<_>>();
    for handle in handles {
        handle.join().expect("join");
    }

    let listed = store.list().expect("list");
    assert_eq!(listed.len(), workers * per_worker);
    let ids = listed
        .iter()
        .map(|message| message.id.clone())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), workers * per_worker);
}

#[test]
fn concurrent_gateway_claims_do_not_duplicate_messages() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(LocalGatewayStore::new(temp.path()));
    let message_count = 20usize;
    for index in 0..message_count {
        store
            .enqueue(GatewayRoute::new(
                "cli",
                GatewayMessageKind::Task,
                format!("message-{index}"),
                None,
            ))
            .expect("enqueue");
    }

    let workers = 10usize;
    let barrier = Arc::new(Barrier::new(workers));
    let handles = (0..workers)
        .map(|_| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                store
                    .claim_pending(3)
                    .expect("claim")
                    .into_iter()
                    .map(|message| message.id)
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>();

    let mut claimed = Vec::new();
    for handle in handles {
        claimed.extend(handle.join().expect("join"));
    }
    let unique = claimed.iter().cloned().collect::<BTreeSet<_>>();
    assert_eq!(claimed.len(), message_count);
    assert_eq!(unique.len(), message_count);
    assert!(store.pending(10).expect("pending").is_empty());
    assert!(
        store
            .list()
            .expect("list")
            .iter()
            .all(|message| message.status == GatewayMessageStatus::Processing)
    );
}

#[test]
fn gateway_delivers_outbox_records() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let message = store
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Chat,
            "hello",
            None,
        ))
        .expect("message");

    let delivery = store
        .deliver(&message.id, "chat_response", "hi")
        .expect("delivery");
    let deliveries = store.deliveries().expect("deliveries");
    assert_eq!(deliveries, vec![delivery]);
    assert_eq!(store.outbox_path(), temp.path().join("outbox.jsonl"));
}

#[test]
fn gateway_redacts_secret_like_values_before_storage() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let message = store
        .enqueue(GatewayRoute::new(
            "api_key=abc123",
            GatewayMessageKind::Chat,
            "please keep token=abc123 safe",
            Some("profile password=hunter2".into()),
        ))
        .expect("message");

    store
        .record_status(
            &message.id,
            GatewayMessageStatus::Failed,
            "failed sk-test-secret",
        )
        .expect("record");
    store
        .deliver(&message.id, "chat_response", "response token=abc123")
        .expect("delivery");

    let inbox = fs::read_to_string(store.inbox_path()).expect("inbox");
    let outbox = fs::read_to_string(store.outbox_path()).expect("outbox");
    assert!(!inbox.contains("abc123"));
    assert!(!inbox.contains("hunter2"));
    assert!(!inbox.contains("sk-test-secret"));
    assert!(!outbox.contains("abc123"));
    assert!(inbox.contains("[REDACTED_SECRET]"));
    assert!(outbox.contains("[REDACTED_SECRET]"));
}
