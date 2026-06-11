// SPDX-License-Identifier: GPL-3.0-only

mod agent;
mod app;
mod approval;
mod body;
mod chat;
mod code;
mod diagnostics;
mod fs;
mod git;
mod memory;
mod message;
mod persona;
mod policy;
mod rag;
mod relationship;
mod repo;
mod runtime_context;
mod schedule;
mod self_modify;
mod service;
mod skill;
mod task;
mod testing;
mod voice;

use anyhow::Result;

pub(crate) use runtime_context::{
    print_approval_hint, print_skill_result, resolve_agent, resolve_agent_instance,
    session_and_registry, session_and_registry_for_instance, skill_env,
};

#[tokio::main]
async fn main() -> Result<()> {
    app::run().await
}
