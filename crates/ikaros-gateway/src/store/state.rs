// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    GatewayDelivery, GatewayDeliveryStatus, GatewayMessage, GatewayMessageStatus, GatewayPairing,
    GatewayPairingStatus, GatewayRoute,
};
use ikaros_core::{IkarosError, Result};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::PROCESSING_CLAIM_TIMEOUT;

pub(super) fn idempotency_digest_matches(
    message: &GatewayMessage,
    digest: &str,
    route: &GatewayRoute,
) -> bool {
    if message.idempotency_key_digest.as_deref() == Some(digest) {
        return true;
    }
    message.idempotency_key_digest.is_none()
        && route
            .idempotency_key
            .as_deref()
            .is_some_and(|key| !key.contains("[REDACTED_SECRET]"))
        && message.idempotency_key == route.idempotency_key
}

pub(super) fn pairing_matches_route(pairing: &GatewayPairing, route: &GatewayRoute) -> bool {
    pairing.status != GatewayPairingStatus::Revoked
        && pairing
            .source
            .trim()
            .eq_ignore_ascii_case(route_channel(route))
        && pairing.account.as_deref() == route_account(route)
        && Some(pairing.peer.as_str()) == route_peer(route)
}

pub(super) fn route_channel(route: &GatewayRoute) -> &str {
    route
        .session_source
        .as_ref()
        .map(|source| source.channel.as_str())
        .unwrap_or(route.source.as_str())
}

pub(super) fn route_account(route: &GatewayRoute) -> Option<&str> {
    route.session_source.as_ref()?.account.as_deref()
}

pub(super) fn route_peer(route: &GatewayRoute) -> Option<&str> {
    route.session_source.as_ref()?.peer.as_deref()
}

pub(super) fn lease_update_allowed(
    message: &GatewayMessage,
    claim: Option<&GatewayMessage>,
) -> bool {
    if gateway_terminal_status(&message.status) {
        return false;
    }
    if let Some(claim) = claim {
        return message.id == claim.id
            && message.status == GatewayMessageStatus::Processing
            && message.lease_owner == claim.lease_owner
            && message.attempt_count == claim.attempt_count;
    }
    !matches!(message.status, GatewayMessageStatus::Processing) || message.lease_owner.is_none()
}

pub(super) fn gateway_terminal_status(status: &GatewayMessageStatus) -> bool {
    matches!(
        status,
        GatewayMessageStatus::Processed
            | GatewayMessageStatus::Failed
            | GatewayMessageStatus::Cancelled
            | GatewayMessageStatus::DeadLettered
    )
}

pub(super) fn sort_messages(messages: &mut [GatewayMessage]) {
    messages.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
}

pub(super) fn sort_deliveries(deliveries: &mut [GatewayDelivery]) {
    deliveries.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.id.cmp(&right.id))
    });
}

pub(super) fn sort_pairings(pairings: &mut [GatewayPairing]) {
    pairings.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.code.cmp(&right.code))
    });
}

pub(super) fn claimable_delivery(delivery: &GatewayDelivery, now: OffsetDateTime) -> bool {
    match delivery.status {
        GatewayDeliveryStatus::Pending => delivery
            .next_attempt_at
            .as_deref()
            .and_then(|deadline| OffsetDateTime::parse(deadline, &Rfc3339).ok())
            .is_none_or(|deadline| now >= deadline),
        GatewayDeliveryStatus::Processing => delivery_claim_expired(delivery, now),
        GatewayDeliveryStatus::Delivered | GatewayDeliveryStatus::DeadLettered => false,
    }
}

pub(super) fn delivery_claim_expired(delivery: &GatewayDelivery, now: OffsetDateTime) -> bool {
    delivery
        .lease_expires_at
        .as_deref()
        .and_then(|deadline| OffsetDateTime::parse(deadline, &Rfc3339).ok())
        .is_some_and(|lease_expires_at| now >= lease_expires_at)
}

pub(super) fn delivery_update_allowed(delivery: &GatewayDelivery, claim: &GatewayDelivery) -> bool {
    delivery.id == claim.id
        && delivery.status == GatewayDeliveryStatus::Processing
        && delivery.lease_owner == claim.lease_owner
        && delivery.attempt_count == claim.attempt_count
}

pub(super) fn claimable_message(message: &GatewayMessage, now: OffsetDateTime) -> bool {
    match message.status {
        GatewayMessageStatus::Pending => true,
        GatewayMessageStatus::Processing => processing_claim_expired(message, now),
        GatewayMessageStatus::Processed
        | GatewayMessageStatus::Failed
        | GatewayMessageStatus::Cancelled
        | GatewayMessageStatus::DeadLettered => false,
    }
}

pub(super) fn processing_claim_expired(message: &GatewayMessage, now: OffsetDateTime) -> bool {
    if let Some(deadline) = message.lease_expires_at.as_deref() {
        return OffsetDateTime::parse(deadline, &Rfc3339)
            .map(|lease_expires_at| now >= lease_expires_at)
            .unwrap_or(false);
    }
    OffsetDateTime::parse(&message.updated_at, &Rfc3339)
        .map(|updated_at| now - updated_at >= PROCESSING_CLAIM_TIMEOUT)
        .unwrap_or(false)
}

pub(super) fn format_rfc3339(value: OffsetDateTime) -> Result<String> {
    value.format(&Rfc3339).map_err(|source| {
        IkarosError::Message(format!(
            "failed to format gateway lease timestamp: {source}"
        ))
    })
}
