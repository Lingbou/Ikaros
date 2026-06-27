// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    GatewayDelivery, GatewayDeliveryStatus, GatewayMessage, GatewayMessageStatus, GatewayPairing,
    GatewayPairingStatus, GatewayRoute,
};
use ikaros_core::{Result, now_rfc3339, redact_secrets};
use std::path::{Path, PathBuf};
use time::{Duration, OffsetDateTime};

mod jsonl;
mod state;

use jsonl::{read_jsonl, with_jsonl_lock, write_jsonl};
use state::*;

const PROCESSING_CLAIM_TIMEOUT: Duration = Duration::minutes(15);
#[derive(Debug, Clone)]
pub struct LocalGatewayStore {
    inbox_path: PathBuf,
    outbox_path: PathBuf,
    pairings_path: PathBuf,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct GatewayPairingReport {
    pub code: String,
    pub source: String,
    pub account: Option<String>,
    pub peer: String,
    pub status: GatewayPairingStatus,
    pub created_at: String,
    pub paired_at: Option<String>,
    pub revoked_at: Option<String>,
}

impl LocalGatewayStore {
    pub fn new(gateway_dir: impl Into<PathBuf>) -> Self {
        let gateway_dir = gateway_dir.into();
        Self {
            inbox_path: gateway_dir.join("inbox.jsonl"),
            outbox_path: gateway_dir.join("outbox.jsonl"),
            pairings_path: gateway_dir.join("pairings.jsonl"),
        }
    }

    pub fn from_files(inbox_path: impl Into<PathBuf>, outbox_path: impl Into<PathBuf>) -> Self {
        let inbox_path = inbox_path.into();
        let pairings_path = inbox_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("pairings.jsonl");
        Self {
            inbox_path,
            outbox_path: outbox_path.into(),
            pairings_path,
        }
    }

    pub fn inbox_path(&self) -> &Path {
        &self.inbox_path
    }

    pub fn outbox_path(&self) -> &Path {
        &self.outbox_path
    }

    pub fn pairings_path(&self) -> &Path {
        &self.pairings_path
    }

    pub fn enqueue(&self, route: GatewayRoute) -> Result<GatewayMessage> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages: Vec<GatewayMessage> = read_jsonl(&self.inbox_path)?;
            if let Some(digest) = route.idempotency_key_digest.as_deref() {
                if let Some(existing) = messages
                    .iter()
                    .find(|message| idempotency_digest_matches(message, digest, &route))
                {
                    return Ok(existing.clone());
                }
            } else if let Some(key) = route.idempotency_key.as_deref() {
                if let Some(existing) = messages
                    .iter()
                    .find(|message| message.idempotency_key.as_deref() == Some(key))
                {
                    return Ok(existing.clone());
                }
            }
            let message = GatewayMessage::new(route)?;
            messages.push(message.clone());
            write_jsonl(&self.inbox_path, &messages)?;
            Ok(message)
        })
    }

    pub fn list(&self) -> Result<Vec<GatewayMessage>> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages = read_jsonl(&self.inbox_path)?;
            sort_messages(&mut messages);
            Ok(messages)
        })
    }

    pub fn pending(&self, limit: usize) -> Result<Vec<GatewayMessage>> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages = self
                .read_messages()?
                .into_iter()
                .filter(|message| message.status == GatewayMessageStatus::Pending)
                .collect::<Vec<_>>();
            sort_messages(&mut messages);
            messages.truncate(limit);
            Ok(messages)
        })
    }

    pub fn claim_pending(&self, limit: usize) -> Result<Vec<GatewayMessage>> {
        self.claim_pending_with_owner(limit, "local-gateway-worker")
    }

    pub fn claim_pending_with_owner(
        &self,
        limit: usize,
        owner: impl Into<String>,
    ) -> Result<Vec<GatewayMessage>> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages = self.read_messages()?;
            sort_messages(&mut messages);
            let now_at = OffsetDateTime::now_utc();
            let now = now_rfc3339()?;
            let lease_expires_at = format_rfc3339(now_at + PROCESSING_CLAIM_TIMEOUT)?;
            let owner = redact_secrets(&owner.into());
            let mut claimed = Vec::new();
            for message in &mut messages {
                if claimed.len() >= limit {
                    break;
                }
                if claimable_message(message, now_at) {
                    message.status = GatewayMessageStatus::Processing;
                    message.attempt_count = message.attempt_count.saturating_add(1);
                    message.lease_owner = Some(owner.clone());
                    message.lease_expires_at = Some(lease_expires_at.clone());
                    message.processed_at = None;
                    message.dead_lettered_at = None;
                    message.updated_at = now.clone();
                    claimed.push(message.clone());
                }
            }
            if !claimed.is_empty() {
                write_jsonl(&self.inbox_path, &messages)?;
            }
            Ok(claimed)
        })
    }

    pub fn record_status(
        &self,
        id: &str,
        status: GatewayMessageStatus,
        summary: impl Into<String>,
    ) -> Result<Option<GatewayMessage>> {
        self.record_status_with_claim(id, None, status, summary)
    }

    pub fn record_status_for_claim(
        &self,
        claim: &GatewayMessage,
        status: GatewayMessageStatus,
        summary: impl Into<String>,
    ) -> Result<Option<GatewayMessage>> {
        self.record_status_with_claim(&claim.id, Some(claim), status, summary)
    }

    fn record_status_with_claim(
        &self,
        id: &str,
        claim: Option<&GatewayMessage>,
        status: GatewayMessageStatus,
        summary: impl Into<String>,
    ) -> Result<Option<GatewayMessage>> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages = self.read_messages()?;
            let now = now_rfc3339()?;
            let summary = redact_secrets(&summary.into());
            let mut updated = None;
            for message in &mut messages {
                if message.id == id {
                    if !lease_update_allowed(message, claim) {
                        break;
                    }
                    message.status = status.clone();
                    message.summary = Some(summary.clone());
                    message.processed_at = Some(now.clone());
                    message.lease_owner = None;
                    message.lease_expires_at = None;
                    if matches!(
                        message.status,
                        GatewayMessageStatus::Failed | GatewayMessageStatus::DeadLettered
                    ) {
                        message.last_error = Some(summary.clone());
                    }
                    if message.status == GatewayMessageStatus::DeadLettered {
                        message.dead_lettered_at = Some(now.clone());
                    }
                    message.updated_at = now.clone();
                    updated = Some(message.clone());
                    break;
                }
            }
            self.write_messages(&messages)?;
            Ok(updated)
        })
    }

    pub fn record_failure(
        &self,
        id: &str,
        error: impl Into<String>,
        max_attempts: u32,
    ) -> Result<Option<GatewayMessage>> {
        self.record_failure_with_claim(id, None, error, max_attempts)
    }

    pub fn record_failure_for_claim(
        &self,
        claim: &GatewayMessage,
        error: impl Into<String>,
        max_attempts: u32,
    ) -> Result<Option<GatewayMessage>> {
        self.record_failure_with_claim(&claim.id, Some(claim), error, max_attempts)
    }

    fn record_failure_with_claim(
        &self,
        id: &str,
        claim: Option<&GatewayMessage>,
        error: impl Into<String>,
        max_attempts: u32,
    ) -> Result<Option<GatewayMessage>> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages = self.read_messages()?;
            let now = now_rfc3339()?;
            let error = redact_secrets(&error.into());
            let max_attempts = max_attempts.max(1);
            let mut updated = None;
            for message in &mut messages {
                if message.id == id {
                    if !lease_update_allowed(message, claim) {
                        break;
                    }
                    message.summary = Some(error.clone());
                    message.last_error = Some(error.clone());
                    message.lease_owner = None;
                    message.lease_expires_at = None;
                    message.updated_at = now.clone();
                    if message.attempt_count >= max_attempts {
                        message.status = GatewayMessageStatus::DeadLettered;
                        message.processed_at = Some(now.clone());
                        message.dead_lettered_at = Some(now.clone());
                    } else {
                        message.status = GatewayMessageStatus::Pending;
                        message.processed_at = None;
                        message.dead_lettered_at = None;
                    }
                    updated = Some(message.clone());
                    break;
                }
            }
            self.write_messages(&messages)?;
            Ok(updated)
        })
    }

    pub fn cancel(&self, id: &str, reason: impl Into<String>) -> Result<Option<GatewayMessage>> {
        with_jsonl_lock(&self.inbox_path, || {
            let mut messages = self.read_messages()?;
            let now = now_rfc3339()?;
            let reason = redact_secrets(&reason.into());
            let mut updated = None;
            for message in &mut messages {
                if message.id == id {
                    if gateway_terminal_status(&message.status) {
                        break;
                    }
                    message.status = GatewayMessageStatus::Cancelled;
                    message.summary = Some(reason.clone());
                    message.last_error = Some(reason.clone());
                    message.lease_owner = None;
                    message.lease_expires_at = None;
                    message.processed_at = Some(now.clone());
                    message.dead_lettered_at = None;
                    message.updated_at = now.clone();
                    updated = Some(message.clone());
                    break;
                }
            }
            self.write_messages(&messages)?;
            Ok(updated)
        })
    }

    pub fn delete(&self, id: &str) -> Result<bool> {
        with_jsonl_lock(&self.inbox_path, || {
            let messages = self.read_messages()?;
            let before = messages.len();
            let retained = messages
                .into_iter()
                .filter(|message| message.id != id)
                .collect::<Vec<_>>();
            self.write_messages(&retained)?;
            Ok(retained.len() != before)
        })
    }

    pub fn create_pairing(
        &self,
        source: impl Into<String>,
        account: Option<&str>,
        peer: impl Into<String>,
    ) -> Result<GatewayPairing> {
        with_jsonl_lock(&self.pairings_path, || {
            let mut pairings = self.read_pairings()?;
            let pairing = GatewayPairing::new(source, account.map(ToOwned::to_owned), peer)?;
            pairings.push(pairing.clone());
            self.write_pairings(&pairings)?;
            Ok(pairing)
        })
    }

    pub fn pairings(&self) -> Result<Vec<GatewayPairing>> {
        with_jsonl_lock(&self.pairings_path, || {
            let mut pairings = self.read_pairings()?;
            sort_pairings(&mut pairings);
            Ok(pairings)
        })
    }

    pub fn redacted_pairing_reports(&self) -> Result<Vec<GatewayPairingReport>> {
        Ok(self
            .pairings()?
            .into_iter()
            .map(|pairing| GatewayPairingReport {
                code: "[REDACTED_PAIRING_CODE]".into(),
                source: redact_secrets(&pairing.source),
                account: pairing.account.as_deref().map(redact_secrets),
                peer: redact_secrets(&pairing.peer),
                status: pairing.status,
                created_at: pairing.created_at,
                paired_at: pairing.paired_at,
                revoked_at: pairing.revoked_at,
            })
            .collect())
    }

    pub fn route_has_paired_peer(&self, route: &GatewayRoute) -> Result<bool> {
        with_jsonl_lock(&self.pairings_path, || {
            let pairings = self.read_pairings()?;
            Ok(pairings.iter().any(|pairing| {
                pairing.status == GatewayPairingStatus::Paired
                    && pairing_matches_route(pairing, route)
            }))
        })
    }

    pub fn confirm_pairing_for_route(
        &self,
        route: &GatewayRoute,
        code: &str,
    ) -> Result<Option<GatewayPairing>> {
        with_jsonl_lock(&self.pairings_path, || {
            let mut pairings = self.read_pairings()?;
            let now = now_rfc3339()?;
            let mut confirmed = None;
            for pairing in &mut pairings {
                if pairing.status == GatewayPairingStatus::Pending
                    && pairing.code == code
                    && pairing_matches_route(pairing, route)
                {
                    pairing.status = GatewayPairingStatus::Paired;
                    pairing.paired_at = Some(now.clone());
                    pairing.revoked_at = None;
                    confirmed = Some(pairing.clone());
                    break;
                }
            }
            if confirmed.is_some() {
                self.write_pairings(&pairings)?;
            }
            Ok(confirmed)
        })
    }

    pub fn deliver(
        &self,
        message_id: impl Into<String>,
        kind: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<GatewayDelivery> {
        with_jsonl_lock(&self.outbox_path, || {
            let mut deliveries = self.read_deliveries()?;
            let delivery = GatewayDelivery::new(message_id, kind, content)?;
            deliveries.push(delivery.clone());
            self.write_deliveries(&deliveries)?;
            Ok(delivery)
        })
    }

    pub fn deliveries(&self) -> Result<Vec<GatewayDelivery>> {
        with_jsonl_lock(&self.outbox_path, || {
            let mut deliveries = self.read_deliveries()?;
            sort_deliveries(&mut deliveries);
            Ok(deliveries)
        })
    }

    pub fn claim_pending_deliveries_with_owner(
        &self,
        limit: usize,
        owner: impl Into<String>,
    ) -> Result<Vec<GatewayDelivery>> {
        with_jsonl_lock(&self.outbox_path, || {
            let mut deliveries = self.read_deliveries()?;
            sort_deliveries(&mut deliveries);
            let now_at = OffsetDateTime::now_utc();
            let now = now_rfc3339()?;
            let lease_expires_at = format_rfc3339(now_at + PROCESSING_CLAIM_TIMEOUT)?;
            let owner = redact_secrets(&owner.into());
            let mut claimed = Vec::new();
            for delivery in &mut deliveries {
                if claimed.len() >= limit {
                    break;
                }
                if claimable_delivery(delivery, now_at) {
                    delivery.status = GatewayDeliveryStatus::Processing;
                    delivery.attempt_count = delivery.attempt_count.saturating_add(1);
                    delivery.lease_owner = Some(owner.clone());
                    delivery.lease_expires_at = Some(lease_expires_at.clone());
                    delivery.next_attempt_at = None;
                    delivery.delivered_at = None;
                    delivery.dead_lettered_at = None;
                    delivery.summary = Some(format!("claimed at {now}"));
                    claimed.push(delivery.clone());
                }
            }
            if !claimed.is_empty() {
                self.write_deliveries(&deliveries)?;
            }
            Ok(claimed)
        })
    }

    pub fn record_delivery_success_for_claim(
        &self,
        claim: &GatewayDelivery,
        summary: impl Into<String>,
    ) -> Result<Option<GatewayDelivery>> {
        with_jsonl_lock(&self.outbox_path, || {
            let mut deliveries = self.read_deliveries()?;
            let now = now_rfc3339()?;
            let summary = redact_secrets(&summary.into());
            let mut updated = None;
            for delivery in &mut deliveries {
                if delivery.id == claim.id {
                    if !delivery_update_allowed(delivery, claim) {
                        break;
                    }
                    delivery.status = GatewayDeliveryStatus::Delivered;
                    delivery.summary = Some(summary.clone());
                    delivery.lease_owner = None;
                    delivery.lease_expires_at = None;
                    delivery.next_attempt_at = None;
                    delivery.delivered_at = Some(now.clone());
                    delivery.dead_lettered_at = None;
                    updated = Some(delivery.clone());
                    break;
                }
            }
            self.write_deliveries(&deliveries)?;
            Ok(updated)
        })
    }

    pub fn delivery_claim_by_owner(&self, id: &str, lease_owner: &str) -> Result<GatewayDelivery> {
        let expected_owner = redact_secrets(lease_owner);
        let Some(delivery) = self
            .deliveries()?
            .into_iter()
            .find(|delivery| delivery.id == id)
        else {
            return Err(ikaros_core::IkarosError::Message(format!(
                "delivery not found: {}",
                redact_secrets(id)
            )));
        };
        if delivery.status != GatewayDeliveryStatus::Processing {
            return Err(ikaros_core::IkarosError::Message(format!(
                "delivery is not processing: id={} status={:?}",
                redact_secrets(&delivery.id),
                delivery.status
            )));
        }
        if delivery.lease_owner.as_deref() != Some(expected_owner.as_str()) {
            return Err(ikaros_core::IkarosError::Message(format!(
                "delivery lease owner mismatch: id={} expected_owner={}",
                redact_secrets(&delivery.id),
                expected_owner
            )));
        }
        Ok(delivery)
    }

    pub fn record_delivery_failure_for_claim(
        &self,
        claim: &GatewayDelivery,
        error: impl Into<String>,
        max_attempts: u32,
        backoff_seconds: u64,
    ) -> Result<Option<GatewayDelivery>> {
        with_jsonl_lock(&self.outbox_path, || {
            let mut deliveries = self.read_deliveries()?;
            let now_at = OffsetDateTime::now_utc();
            let now = now_rfc3339()?;
            let error = redact_secrets(&error.into());
            let max_attempts = max_attempts.max(1);
            let backoff = Duration::seconds(backoff_seconds.min(i64::MAX as u64) as i64);
            let next_attempt_at = format_rfc3339(now_at + backoff)?;
            let mut updated = None;
            for delivery in &mut deliveries {
                if delivery.id == claim.id {
                    if !delivery_update_allowed(delivery, claim) {
                        break;
                    }
                    delivery.last_error = Some(error.clone());
                    delivery.summary = Some(error.clone());
                    delivery.lease_owner = None;
                    delivery.lease_expires_at = None;
                    if delivery.attempt_count >= max_attempts {
                        delivery.status = GatewayDeliveryStatus::DeadLettered;
                        delivery.next_attempt_at = None;
                        delivery.dead_lettered_at = Some(now.clone());
                    } else {
                        delivery.status = GatewayDeliveryStatus::Pending;
                        delivery.next_attempt_at = Some(next_attempt_at.clone());
                        delivery.dead_lettered_at = None;
                    }
                    updated = Some(delivery.clone());
                    break;
                }
            }
            self.write_deliveries(&deliveries)?;
            Ok(updated)
        })
    }

    fn read_messages(&self) -> Result<Vec<GatewayMessage>> {
        read_jsonl(&self.inbox_path)
    }

    fn write_messages(&self, messages: &[GatewayMessage]) -> Result<()> {
        write_jsonl(&self.inbox_path, messages)
    }

    fn read_deliveries(&self) -> Result<Vec<GatewayDelivery>> {
        read_jsonl(&self.outbox_path)
    }

    fn write_deliveries(&self, deliveries: &[GatewayDelivery]) -> Result<()> {
        write_jsonl(&self.outbox_path, deliveries)
    }

    fn read_pairings(&self) -> Result<Vec<GatewayPairing>> {
        read_jsonl(&self.pairings_path)
    }

    fn write_pairings(&self, pairings: &[GatewayPairing]) -> Result<()> {
        write_jsonl(&self.pairings_path, pairings)
    }
}
