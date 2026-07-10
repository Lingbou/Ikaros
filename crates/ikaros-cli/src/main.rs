// SPDX-License-Identifier: GPL-3.0-only

mod agent;
mod app;
mod approval;
mod chat;
mod code;
mod config;
mod debug;
mod diagnostics;
mod fs;
mod git;
mod mcp;
mod memory;
mod persona;
mod policy;
mod provider;
mod rag;
mod relationship;
mod repo;
mod runtime_context;
mod skill;
mod task;
mod testing;

use anyhow::Result;

pub(crate) use runtime_context::{print_approval_hint, print_skill_result, session_and_registry};

#[tokio::main]
async fn main() -> Result<()> {
    app::run().await
}
