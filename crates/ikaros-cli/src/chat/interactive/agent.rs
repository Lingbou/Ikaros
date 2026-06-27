// SPDX-License-Identifier: GPL-3.0-only

use crate::resolve_agent_instance;
use anyhow::{Result, anyhow};
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile};
use ikaros_harness::AuditEvent;
use ikaros_models::model_request_options_from_config;
use ikaros_runtime::runtime_execution_env;
use serde_json::json;
use std::path::Path;

use super::provider::build_interactive_model_provider;
use super::{InteractiveChatRuntime, terminal_inline};

pub(super) fn handle_agent_command(
    args: Vec<&str>,
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    let requested = args
        .first()
        .copied()
        .ok_or_else(|| anyhow!("usage: /agent <profile-or-instance>"))?;
    let agent_instance = resolve_agent_instance(config, Some(requested), workspace, &paths.home)?;
    let new_agent = ResolvedAgentProfile {
        name: agent_instance.agent_id.clone(),
        profile: agent_instance.profile.clone(),
    };
    let session = ikaros_harness::ExecutionSession::new_with_agent_instance(
        &agent_instance.workspace,
        &paths.audit_dir,
        &agent_instance,
    )
    .with_execution_env(runtime_execution_env(config, &agent_instance.workspace)?);
    let model_config = agent_instance.model_config(&config.model.default).clone();
    let model_provider = agent_instance
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let request_options = model_request_options_from_config(&model_config)?;
    let provider =
        build_interactive_model_provider(&model_config, &model_provider, paths, &session)?;
    runtime.session = session;
    runtime.session.audit.append(AuditEvent::new(
        "chat_agent_switch",
        None,
        format!("chat agent switched to {}", new_agent.name),
        json!({
            "agent": &new_agent.name,
            "agent_mode": new_agent.mode().as_str(),
            "workspace_writes": new_agent.profile.workspace_writes.as_str(),
            "shell": new_agent.profile.shell.as_str(),
            "network": new_agent.profile.network.as_str(),
        }),
    )?)?;
    runtime.agent = new_agent;
    runtime.agent_id = agent_instance.agent_id;
    runtime.state_dir = agent_instance.state_dir;
    runtime.workspace = agent_instance.workspace;
    runtime.model_config = model_config;
    runtime.model_provider = model_provider;
    runtime.request_options = request_options;
    runtime.provider = provider;
    println!(
        "agent: {} mode={} workspace={} workspace_writes={} shell={} network={} model={}",
        terminal_inline(&runtime.agent.name),
        runtime.agent.mode(),
        terminal_inline(&runtime.workspace.display().to_string()),
        runtime.agent.profile.workspace_writes,
        runtime.agent.profile.shell,
        runtime.agent.profile.network,
        terminal_inline(&runtime.model_config.model)
    );
    Ok(())
}
