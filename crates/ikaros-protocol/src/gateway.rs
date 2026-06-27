// SPDX-License-Identifier: GPL-3.0-only
//! Protocol frames for long-running Ikaros gateway clients and daemons.

use ikaros_core::{IkarosError, Result, now_rfc3339, redact_secrets};
use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt::Write as _};
use uuid::Uuid;

pub const GATEWAY_PROTOCOL_VERSION: &str = "ikaros.gateway.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayFrame {
    pub id: String,
    pub protocol: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    pub source: GatewaySessionSource,
    pub payload: GatewayFramePayload,
}

impl GatewayFrame {
    pub fn connect(identity: GatewayClientIdentity, capabilities: Vec<GatewayCapability>) -> Self {
        Self::new(
            GatewaySessionSource::control(),
            GatewayFramePayload::Connect(GatewayConnect {
                identity,
                capabilities,
            }),
        )
    }

    pub fn request(source: GatewaySessionSource, request: GatewayRequest) -> Self {
        Self::new(source, GatewayFramePayload::Request(request))
    }

    pub fn response(source: GatewaySessionSource, response: GatewayResponse) -> Self {
        Self::new(source, GatewayFramePayload::Response(response))
    }

    pub fn event(source: GatewaySessionSource, event: GatewayEvent) -> Self {
        Self::new(source, GatewayFramePayload::Event(event))
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(redact_secrets(&key.into()));
        self
    }

    fn new(source: GatewaySessionSource, payload: GatewayFramePayload) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            protocol: GATEWAY_PROTOCOL_VERSION.into(),
            idempotency_key: None,
            source: source.redacted(),
            payload: payload.redacted(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GatewayFramePayload {
    Connect(GatewayConnect),
    Request(GatewayRequest),
    Response(GatewayResponse),
    Event(GatewayEvent),
}

impl GatewayFramePayload {
    fn redacted(self) -> Self {
        match self {
            Self::Connect(connect) => Self::Connect(connect.redacted()),
            Self::Request(request) => Self::Request(request.redacted()),
            Self::Response(response) => Self::Response(response.redacted()),
            Self::Event(event) => Self::Event(event.redacted()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayConnect {
    pub identity: GatewayClientIdentity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<GatewayCapability>,
}

impl GatewayConnect {
    fn redacted(self) -> Self {
        Self {
            identity: self.identity.redacted(),
            capabilities: self
                .capabilities
                .into_iter()
                .map(GatewayCapability::redacted)
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayRequest {
    pub kind: GatewayRequestKind,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
}

impl GatewayRequest {
    pub fn chat(content: impl Into<String>) -> Self {
        Self {
            kind: GatewayRequestKind::Chat,
            content: content.into(),
            agent: None,
            reply_to: None,
        }
    }

    pub fn task(content: impl Into<String>) -> Self {
        Self {
            kind: GatewayRequestKind::Task,
            content: content.into(),
            agent: None,
            reply_to: None,
        }
    }

    pub fn with_agent(mut self, agent: impl Into<String>) -> Self {
        self.agent = Some(agent.into());
        self
    }

    fn redacted(self) -> Self {
        Self {
            kind: self.kind,
            content: redact_secrets(&self.content),
            agent: self.agent.map(|agent| redact_secrets(&agent)),
            reply_to: self.reply_to.map(|reply_to| redact_secrets(&reply_to)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GatewayRequestKind {
    Chat,
    Task,
}

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
    Cancelled,
    DeadLettered,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GatewayDeliveryStatus {
    Pending,
    Processing,
    Delivered,
    DeadLettered,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum GatewayPairingStatus {
    Pending,
    Paired,
    Revoked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayPairing {
    pub code: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    pub peer: String,
    pub status: GatewayPairingStatus,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paired_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<String>,
}

impl GatewayPairing {
    pub fn new(
        source: impl Into<String>,
        account: Option<String>,
        peer: impl Into<String>,
    ) -> Result<Self> {
        Ok(Self {
            code: Uuid::new_v4().to_string(),
            source: redact_secrets(&source.into()),
            account: account.map(|account| redact_secrets(&account)),
            peer: redact_secrets(&peer.into()),
            status: GatewayPairingStatus::Pending,
            created_at: now_rfc3339()?,
            paired_at: None,
            revoked_at: None,
        })
    }
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub safe_tools: bool,
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
            safe_tools: false,
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

    pub fn with_safe_tools(mut self, safe_tools: bool) -> Self {
        self.safe_tools = safe_tools;
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub safe_tools: bool,
    pub status: GatewayMessageStatus,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attempt_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub processed_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dead_lettered_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
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
            safe_tools: route.safe_tools,
            status: GatewayMessageStatus::Pending,
            attempt_count: 0,
            lease_owner: None,
            lease_expires_at: None,
            created_at: now.clone(),
            updated_at: now,
            processed_at: None,
            dead_lettered_at: None,
            last_error: None,
            summary: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayDelivery {
    pub id: String,
    pub message_id: String,
    pub kind: String,
    pub content: String,
    pub created_at: String,
    #[serde(default = "default_delivery_status")]
    pub status: GatewayDeliveryStatus,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub attempt_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_attempt_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dead_lettered_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
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
            status: GatewayDeliveryStatus::Pending,
            attempt_count: 0,
            lease_owner: None,
            lease_expires_at: None,
            next_attempt_at: None,
            delivered_at: None,
            dead_lettered_at: None,
            last_error: None,
            summary: None,
        })
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GatewayPlatform {
    Generic,
    Webhook,
    Telegram,
    Discord,
    Slack,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn stable_idempotency_digest(key: &str) -> String {
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

fn default_delivery_status() -> GatewayDeliveryStatus {
    GatewayDeliveryStatus::Pending
}

impl GatewayPlatform {
    pub fn parse(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "generic" | "local" => Ok(Self::Generic),
            "webhook" | "http" => Ok(Self::Webhook),
            "telegram" | "tg" => Ok(Self::Telegram),
            "discord" => Ok(Self::Discord),
            "slack" => Ok(Self::Slack),
            other => Err(IkarosError::Message(format!(
                "unsupported gateway adapter platform: {other}"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Generic => "generic",
            Self::Webhook => "webhook",
            Self::Telegram => "telegram",
            Self::Discord => "discord",
            Self::Slack => "slack",
        }
    }

    pub fn default_capabilities(self) -> Vec<GatewayCapability> {
        match self {
            Self::Generic => vec![GatewayCapability::new("chat")],
            Self::Webhook => vec![
                GatewayCapability::new("chat"),
                GatewayCapability::new("task"),
            ],
            Self::Telegram | Self::Discord | Self::Slack => vec![
                GatewayCapability::new("chat"),
                GatewayCapability::new("task"),
                GatewayCapability::new("delivery"),
            ],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayOutboundEnvelope {
    pub platform: GatewayPlatform,
    pub delivery_id: String,
    pub message_id: String,
    pub kind: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

impl GatewayOutboundEnvelope {
    pub fn redacted(
        platform: GatewayPlatform,
        delivery_id: &str,
        message_id: &str,
        kind: &str,
        content: &str,
        source: Option<&GatewaySessionSource>,
    ) -> Self {
        Self {
            platform,
            delivery_id: redact_secrets(delivery_id),
            message_id: redact_secrets(message_id),
            kind: redact_secrets(kind),
            content: redact_secrets(content),
            account: source.and_then(|source| source.account.as_deref().map(redact_secrets)),
            peer: source.and_then(|source| source.peer.as_deref().map(redact_secrets)),
            thread: source.and_then(|source| source.thread.as_deref().map(redact_secrets)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayResponse {
    pub request_id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl GatewayResponse {
    fn redacted(self) -> Self {
        Self {
            request_id: redact_secrets(&self.request_id),
            ok: self.ok,
            content: self.content.map(|content| redact_secrets(&content)),
            error: self.error.map(|error| redact_secrets(&error)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayEvent {
    pub name: String,
    #[serde(default)]
    pub data: serde_json::Value,
}

impl GatewayEvent {
    fn redacted(self) -> Self {
        Self {
            name: redact_secrets(&self.name),
            data: ikaros_core::redact_json(self.data),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GatewayClientIdentity {
    pub client_id: String,
    pub device_id: Option<String>,
    pub account: Option<String>,
    pub display_name: Option<String>,
}

impl GatewayClientIdentity {
    pub fn new(client_id: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            device_id: None,
            account: None,
            display_name: None,
        }
    }

    fn redacted(self) -> Self {
        Self {
            client_id: redact_secrets(&self.client_id),
            device_id: self.device_id.map(|device_id| redact_secrets(&device_id)),
            account: self.account.map(|account| redact_secrets(&account)),
            display_name: self
                .display_name
                .map(|display_name| redact_secrets(&display_name)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayCapability {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

impl GatewayCapability {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: None,
        }
    }

    fn redacted(self) -> Self {
        Self {
            name: redact_secrets(&self.name),
            version: self.version.map(|version| redact_secrets(&version)),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GatewayProtocolPolicy {
    pub allowed_clients: BTreeSet<String>,
    pub allowed_channels: BTreeSet<String>,
    pub required_capabilities: BTreeSet<String>,
}

impl GatewayProtocolPolicy {
    pub fn allow_all_local() -> Self {
        Self::default()
    }

    pub fn with_allowed_clients(mut self, clients: impl IntoIterator<Item = String>) -> Self {
        self.allowed_clients = clients
            .into_iter()
            .map(|client| client.trim().to_owned())
            .filter(|client| !client.is_empty())
            .collect();
        self
    }

    pub fn with_allowed_channels(mut self, channels: impl IntoIterator<Item = String>) -> Self {
        self.allowed_channels = channels
            .into_iter()
            .map(|channel| channel.trim().to_ascii_lowercase())
            .filter(|channel| !channel.is_empty())
            .collect();
        self
    }

    pub fn with_required_capabilities(
        mut self,
        capabilities: impl IntoIterator<Item = String>,
    ) -> Self {
        self.required_capabilities = capabilities
            .into_iter()
            .map(|capability| capability.trim().to_owned())
            .filter(|capability| !capability.is_empty())
            .collect();
        self
    }

    pub fn validate_frame(&self, frame: &GatewayFrame) -> Result<()> {
        if frame.protocol != GATEWAY_PROTOCOL_VERSION {
            return Err(IkarosError::Message(format!(
                "unsupported gateway protocol version: {}",
                redact_secrets(&frame.protocol)
            )));
        }
        let channel = frame.source.channel.trim().to_ascii_lowercase();
        if !self.allowed_channels.is_empty() && !self.allowed_channels.contains(&channel) {
            return Err(IkarosError::Message(format!(
                "gateway channel is not allowed: {}",
                redact_secrets(&frame.source.channel)
            )));
        }
        if let GatewayFramePayload::Connect(connect) = &frame.payload {
            self.validate_connect(connect)?;
        }
        Ok(())
    }

    fn validate_connect(&self, connect: &GatewayConnect) -> Result<()> {
        let client_id = connect.identity.client_id.trim();
        if client_id.is_empty() {
            return Err(IkarosError::Message(
                "gateway connect identity client_id is required".into(),
            ));
        }
        if !self.allowed_clients.is_empty() && !self.allowed_clients.contains(client_id) {
            return Err(IkarosError::Message(format!(
                "gateway client is not allowed: {}",
                redact_secrets(client_id)
            )));
        }
        let capabilities = connect
            .capabilities
            .iter()
            .map(|capability| capability.name.as_str())
            .collect::<BTreeSet<_>>();
        for required in &self.required_capabilities {
            if !capabilities.contains(required.as_str()) {
                return Err(IkarosError::Message(format!(
                    "gateway client missing required capability: {}",
                    redact_secrets(required)
                )));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct GatewaySessionSource {
    pub channel: String,
    pub account: Option<String>,
    pub peer: Option<String>,
    pub thread: Option<String>,
    pub message_id: Option<String>,
}

impl GatewaySessionSource {
    pub fn control() -> Self {
        Self {
            channel: "control".into(),
            account: None,
            peer: None,
            thread: None,
            message_id: None,
        }
    }

    pub fn channel(channel: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            ..Self::default()
        }
    }

    fn redacted(self) -> Self {
        Self {
            channel: redact_secrets(&self.channel),
            account: self.account.map(|account| redact_secrets(&account)),
            peer: self.peer.map(|peer| redact_secrets(&peer)),
            thread: self.thread.map(|thread| redact_secrets(&thread)),
            message_id: self
                .message_id
                .map(|message_id| redact_secrets(&message_id)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_frame_redacts_request_and_keeps_protocol_shape() {
        let source = GatewaySessionSource {
            channel: "slack".into(),
            account: Some("team-token=abc123".into()),
            peer: Some("U1".into()),
            thread: Some("T1".into()),
            message_id: Some("M1".into()),
        };
        let frame = GatewayFrame::request(
            source,
            GatewayRequest::chat("hello api_key=secret").with_agent("build"),
        )
        .with_idempotency_key("idem-token=abc123");
        let raw = serde_json::to_string(&frame).expect("json");

        assert_eq!(frame.protocol, GATEWAY_PROTOCOL_VERSION);
        assert!(raw.contains("request"));
        assert!(!raw.contains("secret"));
        assert!(!raw.contains("abc123"));
        assert!(raw.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn gateway_protocol_policy_rejects_unknown_client_and_channel() {
        let policy = GatewayProtocolPolicy::allow_all_local()
            .with_allowed_clients(["trusted-client".into()])
            .with_allowed_channels(["control".into()])
            .with_required_capabilities(["chat".into()]);
        let frame = GatewayFrame::connect(
            GatewayClientIdentity::new("token=secret-client"),
            vec![GatewayCapability::new("chat")],
        );

        let error = policy.validate_frame(&frame).expect_err("client denied");

        assert!(error.to_string().contains("gateway client is not allowed"));
        assert!(!error.to_string().contains("secret-client"));
    }

    #[test]
    fn gateway_protocol_policy_accepts_allowed_connect() {
        let policy = GatewayProtocolPolicy::allow_all_local()
            .with_allowed_clients(["trusted-client".into()])
            .with_allowed_channels(["control".into()])
            .with_required_capabilities(["chat".into()]);
        let frame = GatewayFrame::connect(
            GatewayClientIdentity::new("trusted-client"),
            vec![GatewayCapability::new("chat")],
        );

        policy.validate_frame(&frame).expect("allowed");
    }
}
