// SPDX-License-Identifier: GPL-3.0-only
//! Local message gateway inbox/outbox metadata for Ikaros.

mod protocol;
mod store;
mod types;

pub use protocol::{
    GATEWAY_PROTOCOL_VERSION, GatewayCapability, GatewayClientIdentity, GatewayConnect,
    GatewayEvent, GatewayFrame, GatewayFramePayload, GatewayProtocolPolicy, GatewayRequest,
    GatewayRequestKind, GatewayResponse, GatewaySessionSource,
};
pub use store::LocalGatewayStore;
pub use types::{
    GatewayDelivery, GatewayMessage, GatewayMessageKind, GatewayMessageStatus, GatewayRoute,
};

#[cfg(test)]
mod tests;
