// SPDX-License-Identifier: GPL-3.0-only
//! Platform adapter descriptors and normalized gateway envelopes.

use crate::{
    GatewayCapability, GatewayClientIdentity, GatewayDelivery, GatewayMessage, GatewayMessageKind,
    GatewayRoute, GatewaySessionSource, LocalGatewayStore,
};
use ikaros_core::{IkarosError, Result, redact_secrets};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GatewayPlatform {
    Generic,
    Webhook,
    Telegram,
    Discord,
    Slack,
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
pub struct GatewayAdapterDescriptor {
    pub id: String,
    pub platform: GatewayPlatform,
    pub display_name: String,
    pub inbound: bool,
    pub outbound: bool,
    pub requires_pairing: bool,
    pub supports_hmac: bool,
    pub safe_tools_default: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<GatewayCapability>,
}

pub fn builtin_gateway_adapters() -> Vec<GatewayAdapterDescriptor> {
    [
        (
            GatewayPlatform::Generic,
            "Generic local adapter",
            false,
            false,
        ),
        (
            GatewayPlatform::Webhook,
            "Loopback webhook adapter",
            false,
            true,
        ),
        (
            GatewayPlatform::Telegram,
            "Telegram adapter descriptor",
            true,
            false,
        ),
        (
            GatewayPlatform::Discord,
            "Discord adapter descriptor",
            true,
            false,
        ),
        (
            GatewayPlatform::Slack,
            "Slack adapter descriptor",
            true,
            false,
        ),
    ]
    .into_iter()
    .map(
        |(platform, display_name, requires_pairing, supports_hmac)| GatewayAdapterDescriptor {
            id: platform.as_str().to_owned(),
            platform,
            display_name: display_name.into(),
            inbound: true,
            outbound: true,
            requires_pairing,
            supports_hmac,
            safe_tools_default: requires_pairing,
            capabilities: platform.default_capabilities(),
        },
    )
    .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GatewayInboundEnvelope {
    pub platform: GatewayPlatform,
    pub content: String,
    pub kind: GatewayMessageKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default)]
    pub safe_tools: bool,
}

impl GatewayInboundEnvelope {
    pub fn to_route(&self) -> GatewayRoute {
        let mut route = GatewayRoute::new(
            self.platform.as_str(),
            self.kind.clone(),
            self.content.clone(),
            self.agent.clone(),
        )
        .with_session_source(GatewaySessionSource {
            channel: self.platform.as_str().to_owned(),
            account: self.account.clone(),
            peer: self.peer.clone(),
            thread: self.thread.clone(),
            message_id: self.message_id.clone(),
        })
        .with_client(
            GatewayClientIdentity {
                client_id: format!("{}-adapter", self.platform.as_str()),
                device_id: None,
                account: self.account.clone(),
                display_name: Some(format!("{} gateway adapter", self.platform.as_str())),
            },
            self.platform.default_capabilities(),
        )
        .with_safe_tools(self.safe_tools);
        if let Some(key) = self.idempotency_key.as_deref() {
            route = route.with_idempotency_key(key);
        }
        route
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
    pub fn from_delivery(
        platform: GatewayPlatform,
        delivery: &GatewayDelivery,
        source: Option<&GatewaySessionSource>,
    ) -> Self {
        Self {
            platform,
            delivery_id: redact_secrets(&delivery.id),
            message_id: redact_secrets(&delivery.message_id),
            kind: redact_secrets(&delivery.kind),
            content: redact_secrets(&delivery.content),
            account: source.and_then(|source| source.account.as_deref().map(redact_secrets)),
            peer: source.and_then(|source| source.peer.as_deref().map(redact_secrets)),
            thread: source.and_then(|source| source.thread.as_deref().map(redact_secrets)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GatewayAdapterEnqueueRequest {
    pub platform: String,
    pub content: String,
    pub kind: GatewayMessageKind,
    pub agent: Option<String>,
    pub account: Option<String>,
    pub peer: Option<String>,
    pub thread: Option<String>,
    pub message_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub safe_tools: bool,
}

#[derive(Debug, Clone)]
pub struct GatewayAdapterEnqueueResult {
    pub platform: GatewayPlatform,
    pub safe_tools: bool,
    pub message: GatewayMessage,
}

pub fn enqueue_gateway_adapter_message(
    store: &LocalGatewayStore,
    request: GatewayAdapterEnqueueRequest,
) -> Result<GatewayAdapterEnqueueResult> {
    let platform = GatewayPlatform::parse(&request.platform)?;
    let descriptor = builtin_gateway_adapters()
        .into_iter()
        .find(|adapter| adapter.platform == platform);
    let safe_tools = request.safe_tools
        || descriptor
            .as_ref()
            .is_some_and(|adapter| adapter.safe_tools_default);
    let envelope = GatewayInboundEnvelope {
        platform,
        content: request.content,
        kind: request.kind,
        agent: request.agent,
        account: request.account,
        peer: request.peer,
        thread: request.thread,
        message_id: request.message_id,
        idempotency_key: request.idempotency_key,
        safe_tools,
    };
    let message = store.enqueue(envelope.to_route())?;
    Ok(GatewayAdapterEnqueueResult {
        platform,
        safe_tools,
        message,
    })
}

#[derive(Debug, Clone)]
pub struct GatewayAdapterRenderDeliveryRequest {
    pub platform: String,
    pub id: String,
    pub message_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct GatewayAdapterRenderDeliveryResult {
    pub platform: GatewayPlatform,
    pub envelope: GatewayOutboundEnvelope,
}

pub fn render_gateway_adapter_delivery(
    store: &LocalGatewayStore,
    request: GatewayAdapterRenderDeliveryRequest,
) -> Result<GatewayAdapterRenderDeliveryResult> {
    let platform = GatewayPlatform::parse(&request.platform)?;
    let deliveries = store.deliveries()?;
    let Some(delivery) = deliveries.iter().find(|delivery| delivery.id == request.id) else {
        return Err(IkarosError::Message(format!(
            "delivery not found: {}",
            redact_secrets(&request.id)
        )));
    };
    let mut source = if let Some(message_id) = request.message_id.as_deref() {
        gateway_message_source_by_id(store, message_id)?
    } else {
        None
    };
    if source.is_none() {
        source = gateway_message_source_by_id(store, &delivery.message_id)?;
    }
    let envelope = GatewayOutboundEnvelope::from_delivery(platform, delivery, source.as_ref());
    Ok(GatewayAdapterRenderDeliveryResult { platform, envelope })
}

pub fn gateway_message_source_by_id(
    store: &LocalGatewayStore,
    message_id: &str,
) -> Result<Option<GatewaySessionSource>> {
    Ok(store
        .list()?
        .into_iter()
        .find(|message| message.id == message_id)
        .and_then(|message| message.session_source))
}
