// SPDX-License-Identifier: GPL-3.0-only

use crate::protocol::{
    GatewayCapability, GatewayClientIdentity, GatewayRequest, GatewayRequestKind,
    GatewaySessionSource,
};
use ikaros_core::{Result, now_rfc3339, redact_secrets};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GatewayMessageKind {
    Chat,
    Task,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GatewayMessageStatus {
    Pending,
    Processing,
    Processed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayRoute {
    pub source: String,
    pub kind: GatewayMessageKind,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_source: Option<GatewaySessionSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_identity: Option<GatewayClientIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<GatewayCapability>,
}

impl GatewayRoute {
    pub fn new(
        source: impl Into<String>,
        kind: GatewayMessageKind,
        content: impl Into<String>,
        agent: Option<String>,
    ) -> Self {
        Self {
            source: redact_secrets(&source.into()),
            kind,
            content: redact_secrets(&content.into()),
            agent: agent.map(|value| redact_secrets(&value)),
            idempotency_key: None,
            idempotency_key_digest: None,
            session_source: None,
            client_identity: None,
            capabilities: Vec::new(),
        }
    }

    pub fn from_protocol_request(source: GatewaySessionSource, request: GatewayRequest) -> Self {
        let kind = match request.kind {
            GatewayRequestKind::Chat => GatewayMessageKind::Chat,
            GatewayRequestKind::Task => GatewayMessageKind::Task,
        };
        Self::new(source.channel.clone(), kind, request.content, request.agent)
            .with_session_source(source)
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        let key = key.into();
        self.idempotency_key_digest = Some(stable_idempotency_digest(&key));
        self.idempotency_key = Some(redact_secrets(&key));
        self
    }

    pub fn with_session_source(mut self, source: GatewaySessionSource) -> Self {
        self.session_source = Some(redacted_source(source));
        self
    }

    pub fn with_client(
        mut self,
        identity: GatewayClientIdentity,
        capabilities: Vec<GatewayCapability>,
    ) -> Self {
        self.client_identity = Some(redacted_identity(identity));
        self.capabilities = capabilities.into_iter().map(redacted_capability).collect();
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayMessage {
    pub id: String,
    pub source: String,
    pub kind: GatewayMessageKind,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_source: Option<GatewaySessionSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_identity: Option<GatewayClientIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<GatewayCapability>,
    pub status: GatewayMessageStatus,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl GatewayMessage {
    pub fn new(route: GatewayRoute) -> Result<Self> {
        let now = now_rfc3339()?;
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            source: route.source,
            kind: route.kind,
            content: route.content,
            agent: route.agent,
            idempotency_key: route.idempotency_key,
            idempotency_key_digest: route.idempotency_key_digest,
            session_source: route.session_source,
            client_identity: route.client_identity,
            capabilities: route.capabilities,
            status: GatewayMessageStatus::Pending,
            created_at: now.clone(),
            updated_at: now,
            processed_at: None,
            summary: None,
        })
    }
}

pub(crate) fn stable_idempotency_digest(key: &str) -> String {
    let mut input = Vec::with_capacity("ikaros.gateway.idempotency.v1\0".len() + key.len());
    input.extend_from_slice(b"ikaros.gateway.idempotency.v1\0");
    input.extend_from_slice(key.as_bytes());
    let digest = digest(&SHA256, &input);
    let mut encoded = String::from("sha256:");
    for byte in digest.as_ref() {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn redacted_source(source: GatewaySessionSource) -> GatewaySessionSource {
    GatewaySessionSource {
        channel: redact_secrets(&source.channel),
        account: source.account.map(|account| redact_secrets(&account)),
        peer: source.peer.map(|peer| redact_secrets(&peer)),
        thread: source.thread.map(|thread| redact_secrets(&thread)),
        message_id: source
            .message_id
            .map(|message_id| redact_secrets(&message_id)),
    }
}

fn redacted_identity(identity: GatewayClientIdentity) -> GatewayClientIdentity {
    GatewayClientIdentity {
        client_id: redact_secrets(&identity.client_id),
        device_id: identity
            .device_id
            .map(|device_id| redact_secrets(&device_id)),
        account: identity.account.map(|account| redact_secrets(&account)),
        display_name: identity
            .display_name
            .map(|display_name| redact_secrets(&display_name)),
    }
}

fn redacted_capability(capability: GatewayCapability) -> GatewayCapability {
    GatewayCapability {
        name: redact_secrets(&capability.name),
        version: capability.version.map(|version| redact_secrets(&version)),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayDelivery {
    pub id: String,
    pub message_id: String,
    pub kind: String,
    pub content: String,
    pub created_at: String,
}

impl GatewayDelivery {
    pub fn new(
        message_id: impl Into<String>,
        kind: impl Into<String>,
        content: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            message_id: message_id.into(),
            kind: redact_secrets(&kind.into()),
            content: redact_secrets(&content.into()),
            created_at: now_rfc3339()?,
        })
    }
}
