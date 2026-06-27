// SPDX-License-Identifier: GPL-3.0-only

mod drain;
mod platform_delivery;
mod types;
mod worker;

pub use drain::{drain_gateway_message, drain_gateway_messages};
pub use platform_delivery::{
    PlatformDeliveryReport, deliver_to_platform, platform_webhook_config_key,
};
pub use types::{GatewayDrainContext, GatewayDrainReport, GatewayWorkerTickReport};
pub use worker::run_gateway_worker_tick;

#[cfg(test)]
mod tests;
