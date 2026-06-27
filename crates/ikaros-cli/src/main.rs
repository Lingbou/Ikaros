// SPDX-License-Identifier: GPL-3.0-only

mod acp;
mod agent;
mod api;
mod app;
mod approval;
mod body;
mod browser;
mod chat;
mod code;
mod config;
mod debug;
mod diagnostics;
mod fs;
mod gateway;
mod git;
mod image;
mod mcp;
mod memory;
mod persona;
mod policy;
mod provider;
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
mod vision;
mod voice;
mod web;

use anyhow::Result;

pub(crate) use runtime_context::{
    print_approval_hint, print_skill_result, resolve_agent, resolve_agent_instance,
    session_and_registry, session_and_registry_for_instance, skill_env,
};

#[tokio::main]
async fn main() -> Result<()> {
    app::run().await
}
