// SPDX-License-Identifier: GPL-3.0-only

mod drain;
mod types;
mod worker;

pub use drain::{drain_gateway_message, drain_gateway_messages};
pub use types::{GatewayDrainContext, GatewayDrainReport, GatewayWorkerTickReport};
pub use worker::run_gateway_worker_tick;

#[cfg(test)]
mod tests;
