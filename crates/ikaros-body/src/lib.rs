// SPDX-License-Identifier: GPL-3.0-only
//! Replaceable body interface contracts.

mod adapter;
mod types;
mod web;

pub use adapter::{BodyAdapter, CliBodyAdapter};
pub use types::{BodyContextSources, BodyEvent, BodyEventKind, BodyFrame, BodyKind, BodyStatus};
pub use web::{DashboardRenderOptions, WebDashboardAdapter};
