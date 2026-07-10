// SPDX-License-Identifier: GPL-3.0-only

//! Agent application facade.
//!
//! This crate owns application-level use-case namespaces and receives host-built
//! dependencies from CLI and surface adapters.

pub mod agent_loop;
pub mod agent_pool;
pub mod body_status;
pub mod chat;
mod emotion;
pub mod relationship;
pub mod session_runner;
pub mod soul;
pub mod task_loop;
