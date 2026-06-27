// SPDX-License-Identifier: GPL-3.0-only
//! Local message gateway inbox/outbox metadata for Ikaros.

mod adapter;
mod protocol;
mod status;
mod store;
mod types;
mod webhook;
mod worker;

pub use adapter::{
    GatewayAdapterDescriptor, GatewayAdapterEnqueueRequest, GatewayAdapterEnqueueResult,
    GatewayAdapterRenderDeliveryRequest, GatewayAdapterRenderDeliveryResult,
    GatewayInboundEnvelope, GatewayOutboundEnvelope, GatewayPlatform, builtin_gateway_adapters,
    enqueue_gateway_adapter_message, gateway_message_source_by_id, render_gateway_adapter_delivery,
};
pub use protocol::{
    GATEWAY_PROTOCOL_VERSION, GatewayCapability, GatewayClientIdentity, GatewayConnect,
    GatewayEvent, GatewayFrame, GatewayFramePayload, GatewayProtocolPolicy, GatewayRequest,
    GatewayRequestKind, GatewayResponse, GatewaySessionSource,
};
pub use status::{
    GatewayDeliveryStatusCounts, GatewayMessageStatusCounts, GatewayStatusSnapshot,
    gateway_lease_is_stale, gateway_message_is_retryable,
};
pub use store::{GatewayPairingReport, LocalGatewayStore};
pub use types::{
    GatewayDelivery, GatewayDeliveryStatus, GatewayMessage, GatewayMessageKind,
    GatewayMessageStatus, GatewayPairing, GatewayPairingStatus, GatewayRoute,
};
pub use webhook::{
    HttpHeaders, MessageWebhookAcl, MessageWebhookHttpResponse, MessageWebhookIngressPolicy,
    MessageWebhookServerConfig, handle_webhook_stream, is_message_route, parse_http_request_line,
    parse_webhook_pairing_code, parse_webhook_payload, read_http_headers, require_loopback_host,
    serve_message_webhook, verify_webhook_signature, webhook_http_response,
    write_webhook_http_response,
};
pub use worker::{
    MESSAGE_WORKER_EVENTS_FILE, MESSAGE_WORKER_LOCK_FILE, MESSAGE_WORKER_STOP_FILE,
    MessageWorkerForensics, MessageWorkerLock, MessageWorkerStaleLockRecovery,
    acquire_message_worker_lock, append_message_worker_event, clear_message_worker_stop_request,
    gateway_worker_events_path, gateway_worker_lock_path, gateway_worker_stop_path,
    latest_nonempty_line, message_daemon_log_path, message_daemon_status_label,
    message_worker_lock_is_stale, message_worker_lock_is_stale_label, pid_is_running,
    redacted_json_field, redacted_message_worker_lock_owner, take_message_worker_stop_request,
    write_message_worker_stop_request,
};

#[cfg(test)]
mod tests;
