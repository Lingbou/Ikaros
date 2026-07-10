// SPDX-License-Identifier: GPL-3.0-only
//! Execution owner for governed local action boundaries.
//!
//! This crate owns the migration target for policy, approvals, audit,
//! sandboxing, process/filesystem/network execution, and tool dispatch.

#[path = "harness/lib.rs"]
pub mod harness;

#[path = "sandbox/lib.rs"]
pub mod sandbox;

#[path = "toolkit/lib.rs"]
pub mod toolkit;

pub mod coding_context;
pub mod coding_runtime;
pub mod coding_workflow;
pub mod diff;
pub mod iteration;
pub mod patch;
pub mod repo;
pub mod review;
pub mod testing;

#[cfg(test)]
mod coding_tests;
