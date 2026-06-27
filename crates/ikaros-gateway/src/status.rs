// SPDX-License-Identifier: GPL-3.0-only
//! Runtime-free status snapshots and gateway queue state helpers.

use crate::{
    GatewayDelivery, GatewayDeliveryStatus, GatewayMessage, GatewayMessageStatus, LocalGatewayStore,
};
use ikaros_core::Result;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GatewayMessageStatusCounts {
    pub pending: usize,
    pub processing: usize,
    pub processed: usize,
    pub failed: usize,
    pub cancelled: usize,
    pub dead_lettered: usize,
}

impl GatewayMessageStatusCounts {
    pub fn from_messages(messages: &[GatewayMessage]) -> Self {
        Self {
            pending: messages
                .iter()
                .filter(|message| message.status == GatewayMessageStatus::Pending)
                .count(),
            processing: messages
                .iter()
                .filter(|message| message.status == GatewayMessageStatus::Processing)
                .count(),
            processed: messages
                .iter()
                .filter(|message| message.status == GatewayMessageStatus::Processed)
                .count(),
            failed: messages
                .iter()
                .filter(|message| message.status == GatewayMessageStatus::Failed)
                .count(),
            cancelled: messages
                .iter()
                .filter(|message| message.status == GatewayMessageStatus::Cancelled)
                .count(),
            dead_lettered: messages
                .iter()
                .filter(|message| message.status == GatewayMessageStatus::DeadLettered)
                .count(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GatewayDeliveryStatusCounts {
    pub pending: usize,
    pub processing: usize,
    pub delivered: usize,
    pub dead_lettered: usize,
}

impl GatewayDeliveryStatusCounts {
    pub fn from_deliveries(deliveries: &[GatewayDelivery]) -> Self {
        Self {
            pending: deliveries
                .iter()
                .filter(|delivery| delivery.status == GatewayDeliveryStatus::Pending)
                .count(),
            processing: deliveries
                .iter()
                .filter(|delivery| delivery.status == GatewayDeliveryStatus::Processing)
                .count(),
            delivered: deliveries
                .iter()
                .filter(|delivery| delivery.status == GatewayDeliveryStatus::Delivered)
                .count(),
            dead_lettered: deliveries
                .iter()
                .filter(|delivery| delivery.status == GatewayDeliveryStatus::DeadLettered)
                .count(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayStatusSnapshot {
    pub messages: Vec<GatewayMessage>,
    pub deliveries: Vec<GatewayDelivery>,
    pub message_counts: GatewayMessageStatusCounts,
    pub delivery_counts: GatewayDeliveryStatusCounts,
}

impl GatewayStatusSnapshot {
    pub fn load(store: &LocalGatewayStore) -> Result<Self> {
        let messages = store.list()?;
        let deliveries = store.deliveries()?;
        let message_counts = GatewayMessageStatusCounts::from_messages(&messages);
        let delivery_counts = GatewayDeliveryStatusCounts::from_deliveries(&deliveries);
        Ok(Self {
            messages,
            deliveries,
            message_counts,
            delivery_counts,
        })
    }
}

pub fn gateway_message_is_retryable(message: &GatewayMessage) -> bool {
    message.status == GatewayMessageStatus::Pending
        && (message.attempt_count > 0 || message.last_error.is_some())
}

pub fn gateway_lease_is_stale(message: &GatewayMessage, now: OffsetDateTime) -> bool {
    if message.status != GatewayMessageStatus::Processing {
        return false;
    }
    message
        .lease_expires_at
        .as_deref()
        .and_then(|deadline| OffsetDateTime::parse(deadline, &Rfc3339).ok())
        .is_some_and(|deadline| now >= deadline)
}
