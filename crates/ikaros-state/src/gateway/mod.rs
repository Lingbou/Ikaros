// SPDX-License-Identifier: GPL-3.0-only
//! Local message gateway inbox/outbox metadata for Ikaros.

mod status;
mod store;
mod worker;

pub use ikaros_protocol::{
    GATEWAY_PROTOCOL_VERSION, GatewayCapability, GatewayClientIdentity, GatewayConnect,
    GatewayDelivery, GatewayDeliveryStatus, GatewayEvent, GatewayFrame, GatewayFramePayload,
    GatewayMessage, GatewayMessageKind, GatewayMessageStatus, GatewayPairing, GatewayPairingStatus,
    GatewayProtocolPolicy, GatewayRequest, GatewayRequestKind, GatewayResponse, GatewayRoute,
    GatewaySessionSource,
};
pub use status::{
    GatewayDeliveryStatusCounts, GatewayMessageStatusCounts, GatewayStatusSnapshot,
    gateway_lease_is_stale, gateway_message_is_retryable,
};
pub use store::{GatewayPairingReport, LocalGatewayStore};
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
