// SPDX-License-Identifier: GPL-3.0-only
//! Gateway adapter and webhook external surface.

mod adapter;
mod webhook;

pub use adapter::{
    GatewayAdapterDescriptor, GatewayAdapterEnqueueRequest, GatewayAdapterEnqueueResult,
    GatewayAdapterRenderDeliveryRequest, GatewayAdapterRenderDeliveryResult,
    GatewayInboundEnvelope, builtin_gateway_adapters, enqueue_gateway_adapter_message,
    render_gateway_adapter_delivery,
};
pub use ikaros_protocol::{
    GATEWAY_PROTOCOL_VERSION, GatewayCapability, GatewayClientIdentity, GatewayConnect,
    GatewayDelivery, GatewayDeliveryStatus, GatewayEvent, GatewayFrame, GatewayFramePayload,
    GatewayMessage, GatewayMessageKind, GatewayMessageStatus, GatewayOutboundEnvelope,
    GatewayPairing, GatewayPairingStatus, GatewayPlatform, GatewayProtocolPolicy, GatewayRequest,
    GatewayRequestKind, GatewayResponse, GatewayRoute, GatewaySessionSource,
};
pub use ikaros_state::gateway::GatewayPairingReport;
pub use webhook::{
    HttpHeaders, MessageWebhookAcl, MessageWebhookHttpResponse, MessageWebhookServerConfig,
    is_message_route, parse_http_request_line, parse_webhook_pairing_code, parse_webhook_payload,
    read_http_headers, require_loopback_host, serve_message_webhook, verify_webhook_signature,
    write_webhook_http_response,
};
