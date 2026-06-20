// SPDX-License-Identifier: GPL-3.0-only
//! Protocol frames for long-running Ikaros gateway clients and daemons.

use ikaros_core::{IkarosError, Result, redact_secrets};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
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
