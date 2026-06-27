// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_core::{AgentInstance, IkarosConfig, IkarosPaths, ResolvedAgentProfile};
use ikaros_harness::ExecutionSession;
use ikaros_skills::SkillEnvironment;
use std::path::Path;

pub(crate) fn session_and_registry(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<(ExecutionSession, ikaros_harness::SkillRegistry)> {
    Ok(ikaros_host::session_and_registry(
        paths,
        workspace,
        agent_override,
    )?)
}

pub(crate) fn resolve_agent(
    config: &IkarosConfig,
    agent_override: Option<&str>,
) -> Result<ResolvedAgentProfile> {
    Ok(ikaros_host::resolve_agent(config, agent_override)?)
}

pub(crate) fn resolve_agent_instance(
    config: &IkarosConfig,
    agent_override: Option<&str>,
    workspace: &Path,
    state_root: &Path,
) -> Result<AgentInstance> {
    Ok(ikaros_host::resolve_agent_instance(
        config,
        agent_override,
        workspace,
        state_root,
    )?)
}

pub(crate) fn session_and_registry_for_instance(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent: &AgentInstance,
) -> Result<(ExecutionSession, ikaros_harness::SkillRegistry)> {
    Ok(ikaros_host::session_and_registry_for_instance(
        paths, config, agent,
    )?)
}

pub(crate) fn skill_env(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
) -> Result<SkillEnvironment> {
    Ok(ikaros_host::skill_environment(paths, workspace, config)?)
}

pub(crate) fn print_skill_result(result: &ikaros_core::ToolResult) -> Result<()> {
    println!("ok: {}", result.ok);
    println!("summary: {}", result.summary);
    println!("{}", serde_json::to_string_pretty(&result.output)?);
    Ok(())
}

pub(crate) fn print_approval_hint(result: &ikaros_core::ToolResult) {
    if let Some(id) = result
        .output
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
    {
        println!("approval: {id}");
        println!("next: ikaros approval approve {id}");
    }
}
