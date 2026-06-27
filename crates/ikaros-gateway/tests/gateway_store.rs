// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::now_rfc3339;
use ikaros_gateway::*;
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
fn gateway_pairing_code_binds_peer_before_future_routes_are_allowed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let pairing = store
        .create_pairing("telegram", Some("bot"), "alice")
        .expect("pairing");
    let route = GatewayRoute::new("telegram", GatewayMessageKind::Chat, "hello", None)
        .with_session_source(GatewaySessionSource {
            channel: "telegram".into(),
            account: Some("bot".into()),
            peer: Some("alice".into()),
            thread: Some("chat-1".into()),
            message_id: Some("msg-1".into()),
        });
    let wrong_peer = GatewayRoute::new("telegram", GatewayMessageKind::Chat, "hello", None)
        .with_session_source(GatewaySessionSource {
            channel: "telegram".into(),
            account: Some("bot".into()),
            peer: Some("bob".into()),
            thread: Some("chat-1".into()),
            message_id: Some("msg-2".into()),
        });

    assert!(!store.route_has_paired_peer(&route).expect("paired"));
    assert!(
        store
            .confirm_pairing_for_route(&wrong_peer, &pairing.code)
            .expect("wrong peer")
            .is_none()
    );
    let confirmed = store
        .confirm_pairing_for_route(&route, &pairing.code)
        .expect("confirm")
        .expect("confirmed");
    assert_eq!(confirmed.peer, "alice");
    assert!(confirmed.paired_at.is_some());
    assert!(store.route_has_paired_peer(&route).expect("paired"));
    assert!(!store.route_has_paired_peer(&wrong_peer).expect("paired"));
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
fn gateway_terminal_messages_cannot_be_overwritten_without_a_live_claim() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let message = store
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Task,
            "terminal state",
            None,
        ))
        .expect("message");

    let processed = store
        .record_status(&message.id, GatewayMessageStatus::Processed, "done")
        .expect("processed")
        .expect("found");
    assert_eq!(processed.status, GatewayMessageStatus::Processed);

    let overwritten = store
        .record_status(&message.id, GatewayMessageStatus::Failed, "late failure")
        .expect("late status update should be ignored");
    assert!(
        overwritten.is_none(),
        "terminal processed message should not accept later status update: {overwritten:?}"
    );
    let late_failure = store
        .record_failure(&message.id, "late failure", 1)
        .expect("late failure update should be ignored");
    assert!(
        late_failure.is_none(),
        "terminal processed message should not accept later failure update: {late_failure:?}"
    );

    let listed = store.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, GatewayMessageStatus::Processed);
    assert_eq!(listed[0].summary.as_deref(), Some("done"));
    assert_eq!(listed[0].last_error, None);
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
fn gateway_claim_records_lease_attempt_and_failure_retry_dead_letter() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let message = store
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Task,
            "retry me",
            None,
        ))
        .expect("message");

    let claimed = store
        .claim_pending_with_owner(1, "worker-a")
        .expect("claim");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, message.id);
    assert_eq!(claimed[0].status, GatewayMessageStatus::Processing);
    assert_eq!(claimed[0].attempt_count, 1);
    assert_eq!(claimed[0].lease_owner.as_deref(), Some("worker-a"));
    assert!(claimed[0].lease_expires_at.is_some());

    let retry = store
        .record_failure_for_claim(&claimed[0], "temporary token=abc123", 2)
        .expect("failure")
        .expect("message");
    assert_eq!(retry.status, GatewayMessageStatus::Pending);
    assert_eq!(retry.attempt_count, 1);
    assert_eq!(
        retry.last_error.as_deref(),
        Some("temporary token=[REDACTED_SECRET]")
    );
    assert!(retry.lease_owner.is_none());
    assert!(retry.lease_expires_at.is_none());
    assert!(retry.dead_lettered_at.is_none());

    let retry_claim = store
        .claim_pending_with_owner(1, "worker-b")
        .expect("retry claim");
    assert_eq!(retry_claim[0].attempt_count, 2);
    assert_eq!(retry_claim[0].lease_owner.as_deref(), Some("worker-b"));

    let dead = store
        .record_failure_for_claim(&retry_claim[0], "permanent failure", 2)
        .expect("dead letter")
        .expect("message");
    assert_eq!(dead.status, GatewayMessageStatus::DeadLettered);
    assert_eq!(dead.attempt_count, 2);
    assert_eq!(dead.last_error.as_deref(), Some("permanent failure"));
    assert!(dead.dead_lettered_at.is_some());
}

#[test]
fn gateway_cancelled_processing_message_cannot_be_completed_by_stale_worker() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let message = store
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Task,
            "cancel me",
            None,
        ))
        .expect("message");

    let claimed = store
        .claim_pending_with_owner(1, "worker-a")
        .expect("claim");
    assert_eq!(claimed[0].id, message.id);
    assert_eq!(claimed[0].status, GatewayMessageStatus::Processing);

    let cancelled = store
        .cancel(&message.id, "operator token=abc123")
        .expect("cancel")
        .expect("cancelled");
    assert_eq!(cancelled.status, GatewayMessageStatus::Cancelled);
    assert_eq!(
        cancelled.summary.as_deref(),
        Some("operator token=[REDACTED_SECRET]")
    );
    assert_eq!(cancelled.lease_owner, None);
    assert_eq!(cancelled.lease_expires_at, None);

    let late_success = store
        .record_status_for_claim(&claimed[0], GatewayMessageStatus::Processed, "late success")
        .expect("late success update should be ignored");
    assert!(late_success.is_none());

    let listed = store.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].status, GatewayMessageStatus::Cancelled);
    assert_eq!(
        listed[0].summary.as_deref(),
        Some("operator token=[REDACTED_SECRET]")
    );
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
fn concurrent_gateway_idempotent_enqueues_create_one_message() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(LocalGatewayStore::new(temp.path()));
    let workers = 12usize;
    let barrier = Arc::new(Barrier::new(workers));
    let handles = (0..workers)
        .map(|worker| {
            let store = Arc::clone(&store);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                store
                    .enqueue(
                        GatewayRoute::new(
                            format!("worker-{worker}"),
                            GatewayMessageKind::Task,
                            format!("message from worker {worker}"),
                            None,
                        )
                        .with_idempotency_key("source=slack thread=abc token=shared-secret"),
                    )
                    .expect("enqueue")
                    .id
            })
        })
        .collect::<Vec<_>>();

    let ids = handles
        .into_iter()
        .map(|handle| handle.join().expect("join"))
        .collect::<BTreeSet<_>>();
    assert_eq!(ids.len(), 1);

    let listed = store.list().expect("list");
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].idempotency_key.as_deref(),
        Some("source=slack thread=abc token=[REDACTED_SECRET]")
    );
    assert!(listed[0].idempotency_key_digest.is_some());

    let inbox = fs::read_to_string(store.inbox_path()).expect("inbox");
    assert!(!inbox.contains("shared-secret"));
    assert!(inbox.contains("sha256:"));
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
fn stale_gateway_worker_cannot_complete_reclaimed_message() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let message = store
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Task,
            "lease protected",
            None,
        ))
        .expect("message");

    let first_claim = store
        .claim_pending_with_owner(1, "worker-a")
        .expect("first claim")
        .pop()
        .expect("claimed");
    assert_eq!(first_claim.id, message.id);
    assert_eq!(first_claim.lease_owner.as_deref(), Some("worker-a"));

    let mut listed = store.list().expect("list");
    let claimed = listed
        .iter_mut()
        .find(|candidate| candidate.id == message.id)
        .expect("listed claimed message");
    claimed.lease_expires_at = Some("2020-01-01T00:00:00Z".into());
    fs::write(
        store.inbox_path(),
        listed
            .iter()
            .map(|message| serde_json::to_string(message).expect("message json"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .expect("force expired lease");

    let second_claim = store
        .claim_pending_with_owner(1, "worker-b")
        .expect("second claim")
        .pop()
        .expect("reclaimed");
    assert_eq!(second_claim.id, message.id);
    assert_eq!(second_claim.lease_owner.as_deref(), Some("worker-b"));
    assert_eq!(second_claim.attempt_count, 2);

    let stale_update = store
        .record_status_for_claim(
            &first_claim,
            GatewayMessageStatus::Processed,
            "stale worker success",
        )
        .expect("stale status update should be ignored without error");
    assert!(stale_update.is_none());

    let listed = store.list().expect("list after stale update");
    let current = listed
        .iter()
        .find(|candidate| candidate.id == message.id)
        .expect("current message");
    assert_eq!(current.status, GatewayMessageStatus::Processing);
    assert_eq!(current.lease_owner.as_deref(), Some("worker-b"));
    assert_eq!(current.attempt_count, 2);

    let stale_failure = store
        .record_failure_for_claim(&first_claim, "stale worker failure", 3)
        .expect("stale failure update should be ignored without error");
    assert!(stale_failure.is_none());
    let listed = store.list().expect("list after stale failure");
    let current = listed
        .iter()
        .find(|candidate| candidate.id == message.id)
        .expect("current message after stale failure");
    assert_eq!(current.status, GatewayMessageStatus::Processing);
    assert_eq!(current.lease_owner.as_deref(), Some("worker-b"));
    assert_eq!(current.attempt_count, 2);

    let owner_update = store
        .record_status_for_claim(
            &second_claim,
            GatewayMessageStatus::Processed,
            "current worker success",
        )
        .expect("current status update")
        .expect("current owner can update");
    assert_eq!(owner_update.status, GatewayMessageStatus::Processed);
    assert!(owner_update.lease_owner.is_none());
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
fn gateway_delivery_queue_claims_retries_and_dead_letters_with_backoff() {
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

    let claimed = store
        .claim_pending_deliveries_with_owner(1, "adapter-a")
        .expect("claim");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, delivery.id);
    assert_eq!(claimed[0].status, GatewayDeliveryStatus::Processing);
    assert_eq!(claimed[0].attempt_count, 1);
    assert_eq!(claimed[0].lease_owner.as_deref(), Some("adapter-a"));
    assert!(claimed[0].lease_expires_at.is_some());

    let retry = store
        .record_delivery_failure_for_claim(&claimed[0], "remote token=abc123", 2, 30)
        .expect("record failure")
        .expect("retry");
    assert_eq!(retry.status, GatewayDeliveryStatus::Pending);
    assert_eq!(retry.attempt_count, 1);
    assert!(retry.next_attempt_at.is_some());
    assert_eq!(
        retry.last_error.as_deref(),
        Some("remote token=[REDACTED_SECRET]")
    );
    assert_eq!(
        store
            .claim_pending_deliveries_with_owner(1, "adapter-b")
            .expect("claim during backoff")
            .len(),
        0
    );

    let mut deliveries = store.deliveries().expect("deliveries");
    let pending = deliveries
        .iter_mut()
        .find(|candidate| candidate.id == delivery.id)
        .expect("delivery listed");
    pending.next_attempt_at = Some("2020-01-01T00:00:00Z".into());
    fs::write(
        store.outbox_path(),
        deliveries
            .iter()
            .map(|delivery| serde_json::to_string(delivery).expect("delivery json"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .expect("force retry ready");

    let second_claim = store
        .claim_pending_deliveries_with_owner(1, "adapter-b")
        .expect("second claim")
        .pop()
        .expect("claimed after backoff");
    assert_eq!(second_claim.attempt_count, 2);
    assert_eq!(second_claim.lease_owner.as_deref(), Some("adapter-b"));

    let dead = store
        .record_delivery_failure_for_claim(&second_claim, "still failing", 2, 30)
        .expect("dead letter")
        .expect("dead letter update");
    assert_eq!(dead.status, GatewayDeliveryStatus::DeadLettered);
    assert!(dead.dead_lettered_at.is_some());
    assert_eq!(dead.lease_owner, None);
    assert_eq!(dead.lease_expires_at, None);
    assert_eq!(
        store
            .claim_pending_deliveries_with_owner(1, "adapter-c")
            .expect("claim after dead letter")
            .len(),
        0
    );
}

#[test]
fn stale_gateway_delivery_worker_cannot_complete_reclaimed_delivery() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let delivery = store
        .deliver("message-one", "chat_response", "hi")
        .expect("delivery");

    let first_claim = store
        .claim_pending_deliveries_with_owner(1, "adapter-a")
        .expect("first claim")
        .pop()
        .expect("claimed");
    assert_eq!(first_claim.id, delivery.id);

    let mut deliveries = store.deliveries().expect("deliveries");
    let current = deliveries
        .iter_mut()
        .find(|candidate| candidate.id == delivery.id)
        .expect("delivery listed");
    current.lease_expires_at = Some("2020-01-01T00:00:00Z".into());
    fs::write(
        store.outbox_path(),
        deliveries
            .iter()
            .map(|delivery| serde_json::to_string(delivery).expect("delivery json"))
            .collect::<Vec<_>>()
            .join("\n")
            + "\n",
    )
    .expect("force expired lease");

    let second_claim = store
        .claim_pending_deliveries_with_owner(1, "adapter-b")
        .expect("second claim")
        .pop()
        .expect("reclaimed");
    assert_eq!(second_claim.attempt_count, 2);
    assert_eq!(second_claim.lease_owner.as_deref(), Some("adapter-b"));

    let stale_success = store
        .record_delivery_success_for_claim(&first_claim, "late success")
        .expect("stale success update should be ignored");
    assert!(stale_success.is_none());
    let current = store
        .deliveries()
        .expect("deliveries")
        .into_iter()
        .find(|candidate| candidate.id == delivery.id)
        .expect("current delivery");
    assert_eq!(current.status, GatewayDeliveryStatus::Processing);
    assert_eq!(current.lease_owner.as_deref(), Some("adapter-b"));

    let success = store
        .record_delivery_success_for_claim(&second_claim, "delivered")
        .expect("success")
        .expect("current owner update");
    assert_eq!(success.status, GatewayDeliveryStatus::Delivered);
    assert!(success.delivered_at.is_some());
    assert_eq!(success.lease_owner, None);
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
