// SPDX-License-Identifier: GPL-3.0-only

use crate::message::{
    message_worker_lock_is_stale_label, print_gateway_worker_forensics, print_gateway_worker_state,
    print_gateway_worker_stop,
};
use anyhow::Result;
use ikaros_core::{IkarosPaths, contains_secret_like};
use ikaros_gateway::{
    GatewayDelivery, GatewayDeliveryStatus, GatewayMessage, GatewayMessageStatus, LocalGatewayStore,
};
use ikaros_runtime::gateway_session_id;
use std::{
    fs,
    path::{Path, PathBuf},
};

use super::super::{WorkbenchCell, WorkbenchCellKind, path_display, terminal_inline};

pub(in crate::chat) fn print_gateway_status(paths: &IkarosPaths) -> Result<()> {
    let store = LocalGatewayStore::new(&paths.gateway_dir);
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
    println!("gateway_inbox: {}", path_display(store.inbox_path()));
    println!("gateway_outbox: {}", path_display(store.outbox_path()));
    println!("gateway_pending: {pending}");
    println!("gateway_processing: {processing}");
    println!("gateway_processed: {processed}");
    println!("gateway_failed: {failed}");
    println!("gateway_cancelled: {cancelled}");
    println!("gateway_dead_lettered: {dead_lettered}");
    println!("gateway_deliveries: {}", deliveries.len());
    print_gateway_delivery_state(&deliveries);
    print_gateway_worker_lock(&store);
    print_gateway_worker_stop(&store);
    print_gateway_worker_forensics(&store);
    print_gateway_worker_state(&messages);
    print_gateway_sessions(&messages);
    Ok(())
}

pub(super) fn screen_gateway_status_cell(store: &LocalGatewayStore) -> Result<WorkbenchCell> {
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
    let (delivery_pending, delivery_processing, delivery_delivered, delivery_dead_lettered) =
        gateway_delivery_counts(&deliveries);
    let lock_path = gateway_worker_lock_path(store);
    let (present, stale, owner) = gateway_worker_lock_summary(&lock_path);
    let lock = match present {
        "true" => "present",
        "false" => "absent",
        other => other,
    };
    Ok(WorkbenchCell {
        kind: WorkbenchCellKind::Continuation,
        title: "gateway".into(),
        detail: format!(
            "pending={pending} processing={processing} failed={failed} cancelled={cancelled} dead_lettered={dead_lettered} delivery_pending={delivery_pending} delivery_processing={delivery_processing} delivery_delivered={delivery_delivered} delivery_dead_lettered={delivery_dead_lettered} lock={lock} stale={stale} owner={owner} command=/gateway daemon status status=/gateway worker=/message worker start=/gateway daemon start stop=/gateway daemon stop restart=/gateway daemon restart adapters=/gateway adapter list"
        ),
    })
}

fn print_gateway_delivery_state(deliveries: &[GatewayDelivery]) {
    let (pending, processing, delivered, dead_lettered) = gateway_delivery_counts(deliveries);
    println!(
        "gateway_deliveries_status: pending={pending} processing={processing} delivered={delivered} dead_lettered={dead_lettered}"
    );
}

fn print_gateway_worker_lock(store: &LocalGatewayStore) {
    let path = gateway_worker_lock_path(store);
    let (present, stale, owner) = gateway_worker_lock_summary(&path);
    println!(
        "gateway_worker_lock: present={} stale={} path={} owner={}",
        present,
        stale,
        path_display(&path),
        owner
    );
}

fn gateway_delivery_counts(deliveries: &[GatewayDelivery]) -> (usize, usize, usize, usize) {
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
    (pending, processing, delivered, dead_lettered)
}

fn gateway_worker_lock_path(store: &LocalGatewayStore) -> PathBuf {
    store
        .inbox_path()
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("message-worker.lock")
}

fn gateway_worker_lock_summary(path: &Path) -> (&'static str, &'static str, String) {
    match fs::read_to_string(path) {
        Ok(owner) => {
            let stale = message_worker_lock_is_stale_label(&owner);
            ("true", stale, gateway_worker_lock_owner(&owner))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            ("false", "false", "none".into())
        }
        Err(error) => (
            "unknown",
            "unknown",
            terminal_inline(&format!("unreadable: {error}")),
        ),
    }
}

fn gateway_worker_lock_owner(owner: &str) -> String {
    owner
        .lines()
        .map(|line| match line.split_once('=') {
            Some((key, value)) if contains_secret_like(value) => {
                format!("{}=[REDACTED_SECRET]", terminal_inline(key))
            }
            _ => terminal_inline(line),
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_owned()
}

fn print_gateway_sessions(messages: &[GatewayMessage]) {
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
            terminal_inline(&session_id),
            terminal_inline(source),
            terminal_inline(thread),
            message.status
        );
        println!(
            "  resume: ikaros chat --chat-session {} --message \"...\"",
            terminal_inline(&session_id)
        );
    }
}
