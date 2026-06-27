// SPDX-License-Identifier: GPL-3.0-only
//! OpenAI-compatible HTTP API surface for Ikaros.

mod api;

pub use api::{ApiServeOptions, serve_api};
