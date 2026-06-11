// SPDX-License-Identifier: GPL-3.0-only
//! Local scheduled automation metadata for Ikaros.

mod store;
mod types;

pub use store::LocalScheduleStore;
pub use types::{ScheduleDeliveryTarget, ScheduleRunUpdate, ScheduledJob};

#[cfg(test)]
mod tests;
