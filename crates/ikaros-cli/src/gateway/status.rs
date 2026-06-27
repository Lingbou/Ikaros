// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::gateway) fn print_gateway_status(store: &LocalGatewayStore) -> Result<()> {
    let snapshot = GatewayStatusSnapshot::load(store)?;
    let messages = &snapshot.messages;
    let deliveries = &snapshot.deliveries;
    let message_counts = &snapshot.message_counts;
    println!("gateway_status:");
    println!("gateway_inbox: {}", store.inbox_path().display());
    println!("gateway_outbox: {}", store.outbox_path().display());
    println!("gateway_pending: {}", message_counts.pending);
    println!("gateway_processing: {}", message_counts.processing);
    println!("gateway_processed: {}", message_counts.processed);
    println!("gateway_failed: {}", message_counts.failed);
    println!("gateway_cancelled: {}", message_counts.cancelled);
    println!("gateway_dead_lettered: {}", message_counts.dead_lettered);
    println!("gateway_deliveries: {}", deliveries.len());
    print_gateway_delivery_state(deliveries);
    print_gateway_worker_lock(store);
    print_gateway_worker_stop(store);
    print_gateway_worker_forensics(store);
    print_gateway_worker_state(messages);
    print_gateway_sessions(messages);
    Ok(())
}

pub(in crate::gateway) fn print_gateway_delivery_state(deliveries: &[GatewayDelivery]) {
    let counts = GatewayDeliveryStatusCounts::from_deliveries(deliveries);
    println!(
        "gateway_deliveries_status: pending={} processing={} delivered={} dead_lettered={}",
        counts.pending, counts.processing, counts.delivered, counts.dead_lettered
    );
    for delivery in deliveries
        .iter()
        .filter(|delivery| {
            delivery.status == GatewayDeliveryStatus::Pending && delivery.last_error.is_some()
        })
        .take(5)
    {
        println!(
            "gateway_retryable_delivery: id={} message_id={} attempts={} next_attempt_at={} last_error={}",
            redact_secrets(&delivery.id),
            redact_secrets(&delivery.message_id),
            delivery.attempt_count,
            delivery
                .next_attempt_at
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| "ready".into()),
            delivery
                .last_error
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| "none".into())
        );
    }
    for delivery in deliveries
        .iter()
        .filter(|delivery| delivery.status == GatewayDeliveryStatus::DeadLettered)
        .take(5)
    {
        println!(
            "gateway_dead_lettered_delivery: id={} message_id={} attempts={} dead_lettered_at={} last_error={}",
            redact_secrets(&delivery.id),
            redact_secrets(&delivery.message_id),
            delivery.attempt_count,
            delivery
                .dead_lettered_at
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| "none".into()),
            delivery
                .last_error
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| "none".into())
        );
    }
}
pub(in crate::gateway) fn print_message_daemon_status(store: &LocalGatewayStore) {
    println!(
        "message_daemon_status: {}",
        message_daemon_status_label(store)
    );
}

pub(in crate::gateway) fn print_gateway_worker_lock(store: &LocalGatewayStore) {
    let path = gateway_worker_lock_path(store);
    match fs::read_to_string(&path) {
        Ok(owner) => {
            let stale = message_worker_lock_is_stale_label(&owner);
            let owner = redacted_message_worker_lock_owner(&owner);
            println!(
                "gateway_worker_lock: present=true stale={} path={} owner={}",
                stale,
                path.display(),
                owner
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "gateway_worker_lock: present=false stale=false path={} owner=none",
                path.display()
            );
        }
        Err(error) => {
            println!(
                "gateway_worker_lock: present=unknown stale=unknown path={} owner={}",
                path.display(),
                redact_secrets(&format!("unreadable: {error}"))
            );
        }
    }
}

pub(crate) fn print_gateway_worker_forensics(store: &LocalGatewayStore) {
    let path = gateway_worker_events_path(store);
    let Some(line) = latest_nonempty_line(&path) else {
        println!(
            "gateway_worker_forensics: latest_event=none latest_status=none path={} run_id=none at=none pid=none reason=none",
            path.display()
        );
        return;
    };
    match serde_json::from_str::<serde_json::Value>(&line) {
        Ok(value) => {
            println!(
                "gateway_worker_forensics: latest_event={} latest_status={} path={} run_id={} at={} pid={} reason={}",
                redacted_json_field(&value, "event"),
                redacted_json_field(&value, "status"),
                path.display(),
                redacted_json_field(&value, "run_id"),
                redacted_json_field(&value, "at"),
                redacted_json_field(&value, "pid"),
                redacted_json_field(&value, "reason"),
            );
        }
        Err(error) => {
            println!(
                "gateway_worker_forensics: latest_event=unreadable latest_status=unknown path={} run_id=none at=none pid=none reason={}",
                path.display(),
                redact_secrets(&error.to_string())
            );
        }
    }
}

pub(crate) fn print_gateway_worker_stop(store: &LocalGatewayStore) {
    let path = gateway_worker_stop_path(store);
    match fs::read_to_string(&path) {
        Ok(contents) => match serde_json::from_str::<serde_json::Value>(&contents) {
            Ok(value) => {
                println!(
                    "gateway_worker_stop: requested=true path={} at={} pid={} reason={}",
                    path.display(),
                    redacted_json_field(&value, "at"),
                    redacted_json_field(&value, "pid"),
                    redacted_json_field(&value, "reason")
                );
            }
            Err(error) => {
                println!(
                    "gateway_worker_stop: requested=unknown path={} at=none pid=none reason={}",
                    path.display(),
                    redact_secrets(&error.to_string())
                );
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "gateway_worker_stop: requested=false path={} at=none pid=none reason=none",
                path.display()
            );
        }
        Err(error) => {
            println!(
                "gateway_worker_stop: requested=unknown path={} at=none pid=none reason={}",
                path.display(),
                redact_secrets(&format!("unreadable: {error}"))
            );
        }
    }
}

pub(crate) fn print_gateway_worker_state(messages: &[GatewayMessage]) {
    let now = OffsetDateTime::now_utc();
    let active = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Processing)
        .collect::<Vec<_>>();
    let stale_processing = active
        .iter()
        .filter(|message| gateway_lease_is_stale(message, now))
        .count();
    let retryable = messages
        .iter()
        .filter(|message| gateway_message_is_retryable(message))
        .collect::<Vec<_>>();
    let dead_lettered = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::DeadLettered)
        .collect::<Vec<_>>();
    println!(
        "gateway_worker: processing={} stale_processing={stale_processing} retryable={} dead_lettered={}",
        active.len(),
        retryable.len(),
        dead_lettered.len()
    );
    for message in active.into_iter().take(5) {
        println!(
            "gateway_worker_message: id={} attempts={} lease_owner={} lease_expires_at={} stale={}",
            redact_secrets(&message.id),
            message.attempt_count,
            message
                .lease_owner
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| "none".into()),
            message
                .lease_expires_at
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| "none".into()),
            gateway_lease_is_stale(message, now)
        );
    }
    for message in retryable.into_iter().take(5) {
        println!(
            "gateway_retryable_message: id={} attempts={} last_error={}",
            redact_secrets(&message.id),
            message.attempt_count,
            message
                .last_error
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| "none".into())
        );
    }
    for message in dead_lettered.into_iter().take(5) {
        println!(
            "gateway_dead_lettered_message: id={} attempts={} dead_lettered_at={} last_error={}",
            redact_secrets(&message.id),
            message.attempt_count,
            message
                .dead_lettered_at
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| "none".into()),
            message
                .last_error
                .as_deref()
                .map(redact_secrets)
                .unwrap_or_else(|| "none".into())
        );
    }
}

pub(in crate::gateway) fn print_gateway_sessions(messages: &[GatewayMessage]) {
    // TODO(gateway-refactor): move session-id rendering behind a gateway-owned callback/trait.
    // The current ID algorithm is owned by ikaros-runtime/ikaros-session, and ikaros-gateway
    // must not depend on either crate.
    let mut sessions = messages
        .iter()
        .map(|message| {
            let session_id = gateway_session_id(message);
            (
                session_id.to_string(),
                message.source.as_str(),
                message
                    .session_source
                    .as_ref()
                    .and_then(|source| source.thread.as_deref())
                    .unwrap_or(message.id.as_str()),
                message,
            )
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| left.0.cmp(&right.0));
    sessions.dedup_by(|left, right| left.0 == right.0);
    println!("gateway_sessions: {}", sessions.len());
    for (session_id, source, thread, message) in sessions.into_iter().rev().take(5) {
        println!(
            "gateway_session: session={} source={} thread={} last_status={:?}",
            redact_secrets(&session_id),
            redact_secrets(source),
            redact_secrets(thread),
            message.status
        );
        println!(
            "  resume: ikaros chat --chat-session {} --message \"...\"",
            redact_secrets(&session_id)
        );
    }
}
