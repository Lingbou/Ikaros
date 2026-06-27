// SPDX-License-Identifier: GPL-3.0-only

//! State facade for durable local data boundaries.
//!
//! Callers import owned state capabilities through the named modules below.

pub mod automation;

pub mod gateway;

pub mod memory;

pub mod rag;

pub mod session;
