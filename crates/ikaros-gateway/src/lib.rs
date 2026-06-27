// SPDX-License-Identifier: GPL-3.0-only
//! Local message gateway inbox/outbox metadata for Ikaros.

mod adapter;
mod protocol;
mod store;
mod types;

pub use adapter::{
    GatewayAdapterDescriptor, GatewayInboundEnvelope, GatewayOutboundEnvelope, GatewayPlatform,
    builtin_gateway_adapters,
};
pub use protocol::{
    GATEWAY_PROTOCOL_VERSION, GatewayCapability, GatewayClientIdentity, GatewayConnect,
    GatewayEvent, GatewayFrame, GatewayFramePayload, GatewayProtocolPolicy, GatewayRequest,
    GatewayRequestKind, GatewayResponse, GatewaySessionSource,
};
pub use store::LocalGatewayStore;
pub use types::{
    GatewayDelivery, GatewayDeliveryStatus, GatewayMessage, GatewayMessageKind,
    GatewayMessageStatus, GatewayPairing, GatewayPairingStatus, GatewayRoute,
};

#[cfg(test)]
mod tests;
