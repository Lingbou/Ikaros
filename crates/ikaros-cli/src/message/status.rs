// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::message) fn print_gateway_status(store: &LocalGatewayStore) -> Result<()> {
    let messages = store.list()?;
    let deliveries = store.deliveries()?;
    let pending = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Pending)
        .count();
    let processing = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Processing)
        .count();
    let processed = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Processed)
        .count();
    let failed = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Failed)
        .count();
    let cancelled = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::Cancelled)
        .count();
    let dead_lettered = messages
        .iter()
        .filter(|message| message.status == GatewayMessageStatus::DeadLettered)
        .count();
    println!("gateway_status:");
    println!("gateway_inbox: {}", store.inbox_path().display());
    println!("gateway_outbox: {}", store.outbox_path().display());
    println!("gateway_pending: {pending}");
    println!("gateway_processing: {processing}");
    println!("gateway_processed: {processed}");
    println!("gateway_failed: {failed}");
    println!("gateway_cancelled: {cancelled}");
    println!("gateway_dead_lettered: {dead_lettered}");
    println!("gateway_deliveries: {}", deliveries.len());
    print_gateway_delivery_state(&deliveries);
    print_gateway_worker_lock(store);
    print_gateway_worker_stop(store);
    print_gateway_worker_forensics(store);
    print_gateway_worker_state(&messages);
    print_gateway_sessions(&messages);
    Ok(())
}

pub(in crate::message) fn print_gateway_delivery_state(deliveries: &[GatewayDelivery]) {
    let pending = deliveries
        .iter()
        .filter(|delivery| delivery.status == GatewayDeliveryStatus::Pending)
        .count();
    let processing = deliveries
        .iter()
        .filter(|delivery| delivery.status == GatewayDeliveryStatus::Processing)
        .count();
    let delivered = deliveries
        .iter()
        .filter(|delivery| delivery.status == GatewayDeliveryStatus::Delivered)
        .count();
    let dead_lettered = deliveries
        .iter()
        .filter(|delivery| delivery.status == GatewayDeliveryStatus::DeadLettered)
        .count();
    println!(
        "gateway_deliveries_status: pending={pending} processing={processing} delivered={delivered} dead_lettered={dead_lettered}"
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
pub(in crate::message) fn print_message_daemon_status(store: &LocalGatewayStore) {
    println!(
        "message_daemon_status: {}",
        message_daemon_status_label(store)
    );
}

pub(in crate::message) fn message_daemon_status_label(store: &LocalGatewayStore) -> &'static str {
    let lock_path = gateway_worker_lock_path(store);
    match fs::read_to_string(&lock_path) {
        Ok(owner) => {
            if message_worker_lock_is_stale(&owner) {
                "stale"
            } else {
                "running"
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if gateway_worker_stop_path(store).exists() {
                "stopping"
            } else {
                "stopped"
            }
        }
        Err(_) => "unknown",
    }
}

pub(in crate::message) fn print_gateway_worker_lock(store: &LocalGatewayStore) {
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

pub(in crate::message) fn gateway_worker_lock_path(store: &LocalGatewayStore) -> PathBuf {
    store
        .inbox_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(MESSAGE_WORKER_LOCK_FILE)
}

pub(in crate::message) fn redacted_message_worker_lock_owner(owner: &str) -> String {
    owner
        .lines()
        .map(|line| match line.split_once('=') {
            Some((key, value)) if contains_secret_like(value) => {
                format!("{}=[REDACTED_SECRET]", redact_secrets(key))
            }
            _ => redact_secrets(line),
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned()
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

pub(in crate::message) fn gateway_worker_stop_path(store: &LocalGatewayStore) -> PathBuf {
    store
        .inbox_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(MESSAGE_WORKER_STOP_FILE)
}

pub(in crate::message) fn gateway_worker_events_path(store: &LocalGatewayStore) -> PathBuf {
    store
        .inbox_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(MESSAGE_WORKER_EVENTS_FILE)
}

pub(in crate::message) fn message_daemon_log_path(paths: &IkarosPaths) -> PathBuf {
    paths.gateway_dir.join("message-worker-daemon.log")
}

pub(in crate::message) fn latest_nonempty_line(path: &Path) -> Option<String> {
    fs::read_to_string(path)
        .ok()?
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .map(ToOwned::to_owned)
}

pub(in crate::message) fn redacted_json_field(value: &serde_json::Value, key: &str) -> String {
    let Some(value) = value.get(key) else {
        return "none".into();
    };
    let text = match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    redact_secrets(&text.replace(['\n', '\r'], " "))
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

pub(in crate::message) fn gateway_message_is_retryable(message: &GatewayMessage) -> bool {
    message.status == GatewayMessageStatus::Pending
        && (message.attempt_count > 0 || message.last_error.is_some())
}

pub(in crate::message) fn gateway_lease_is_stale(
    message: &GatewayMessage,
    now: OffsetDateTime,
) -> bool {
    if message.status != GatewayMessageStatus::Processing {
        return false;
    }
    message
        .lease_expires_at
        .as_deref()
        .and_then(|deadline| OffsetDateTime::parse(deadline, &Rfc3339).ok())
        .is_some_and(|deadline| now >= deadline)
}
pub(in crate::message) fn print_gateway_sessions(messages: &[GatewayMessage]) {
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
