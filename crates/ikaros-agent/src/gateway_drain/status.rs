// SPDX-License-Identifier: GPL-3.0-only

use super::types::GatewayDrainReport;
use ikaros_core::{Result, redact_secrets};
use ikaros_protocol::{GatewayMessage, GatewayMessageStatus};
use ikaros_state::gateway::LocalGatewayStore;

const GATEWAY_WORKER_MAX_ATTEMPTS: u32 = 2;

pub(super) fn gateway_status_str(status: &GatewayMessageStatus) -> &'static str {
    match status {
        GatewayMessageStatus::Pending => "pending",
        GatewayMessageStatus::Processing => "processing",
        GatewayMessageStatus::Processed => "processed",
        GatewayMessageStatus::Failed => "failed",
        GatewayMessageStatus::Cancelled => "cancelled",
        GatewayMessageStatus::DeadLettered => "dead_lettered",
    }
}

pub(super) fn record_gateway_status_for_drain(
    store: &LocalGatewayStore,
    message: &GatewayMessage,
    status: GatewayMessageStatus,
    summary: &str,
) -> Result<Option<GatewayMessage>> {
    if message.status == GatewayMessageStatus::Pending {
        store.record_status(&message.id, status, summary)
    } else {
        store.record_status_for_claim(message, status, summary)
    }
}

pub(super) fn record_gateway_failure_for_drain(
    store: &LocalGatewayStore,
    message: &GatewayMessage,
    summary: &str,
) -> Result<Option<GatewayMessage>> {
    if message.status == GatewayMessageStatus::Pending {
        store.record_failure(&message.id, summary, GATEWAY_WORKER_MAX_ATTEMPTS)
    } else {
        store.record_failure_for_claim(message, summary, GATEWAY_WORKER_MAX_ATTEMPTS)
    }
}

pub(super) fn current_gateway_status(
    store: &LocalGatewayStore,
    message_id: &str,
) -> Result<Option<GatewayMessageStatus>> {
    Ok(store
        .list()?
        .into_iter()
        .find(|message| message.id == message_id)
        .map(|message| message.status))
}

pub(super) fn record_gateway_error(
    store: &LocalGatewayStore,
    message: &GatewayMessage,
    kind: &str,
    error: String,
) -> Result<GatewayDrainReport> {
    let summary = redact_secrets(&error);
    let update = record_gateway_failure_for_drain(store, message, &summary)?;
    let status = match update.as_ref().map(|message| message.status.clone()) {
        Some(status) => status,
        None => current_gateway_status(store, &message.id)?.unwrap_or(GatewayMessageStatus::Failed),
    };
    Ok(GatewayDrainReport {
        message_id: message.id.clone(),
        kind: kind.into(),
        status,
        summary,
        context: None,
        provider: None,
        model: None,
        streamed: None,
        stream_chunks: None,
        audit: None,
        model_usage: None,
        update,
        delivery: None,
        task_report: None,
    })
}
