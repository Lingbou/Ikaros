// SPDX-License-Identifier: GPL-3.0-only
//! Platform adapter descriptors and normalized gateway envelopes.

use ikaros_core::{IkarosError, Result, redact_secrets};
use ikaros_protocol::{
    GatewayCapability, GatewayClientIdentity, GatewayMessage, GatewayMessageKind,
    GatewayOutboundEnvelope, GatewayPlatform, GatewayRoute, GatewaySessionSource,
};
use ikaros_state::gateway::LocalGatewayStore;
use serde::{Deserialize, Serialize};
use std::path::Path;

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
    gateway_dir: impl AsRef<Path>,
    request: GatewayAdapterEnqueueRequest,
) -> Result<GatewayAdapterEnqueueResult> {
    let store = LocalGatewayStore::new(gateway_dir.as_ref().to_path_buf());
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
    gateway_dir: impl AsRef<Path>,
    request: GatewayAdapterRenderDeliveryRequest,
) -> Result<GatewayAdapterRenderDeliveryResult> {
    let store = LocalGatewayStore::new(gateway_dir.as_ref().to_path_buf());
    let platform = GatewayPlatform::parse(&request.platform)?;
    let deliveries = store.deliveries()?;
    let Some(delivery) = deliveries.iter().find(|delivery| delivery.id == request.id) else {
        return Err(IkarosError::Message(format!(
            "delivery not found: {}",
            redact_secrets(&request.id)
        )));
    };
    let mut source = if let Some(message_id) = request.message_id.as_deref() {
        gateway_message_source_by_id(&store, message_id)?
    } else {
        None
    };
    if source.is_none() {
        source = gateway_message_source_by_id(&store, &delivery.message_id)?;
    }
    let envelope = GatewayOutboundEnvelope::redacted(
        platform,
        &delivery.id,
        &delivery.message_id,
        &delivery.kind,
        &delivery.content,
        source.as_ref(),
    );
    Ok(GatewayAdapterRenderDeliveryResult { platform, envelope })
}

fn gateway_message_source_by_id(
    store: &LocalGatewayStore,
    message_id: &str,
) -> Result<Option<GatewaySessionSource>> {
    Ok(store
        .list()?
        .into_iter()
        .find(|message| message.id == message_id)
        .and_then(|message| message.session_source))
}
