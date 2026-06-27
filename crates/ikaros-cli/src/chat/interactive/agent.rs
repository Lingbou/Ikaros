// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, anyhow};
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile};
use ikaros_execution::harness::AuditEvent;
use ikaros_host::chat_runtime_services;
use serde_json::json;
use std::path::Path;

use crate::chat::notice::WorkbenchNotice;

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
    let services = chat_runtime_services(paths, config, workspace, Some(requested))?;
    let agent_instance = services.agent_instance;
    let new_agent = ResolvedAgentProfile {
        name: agent_instance.agent_id.clone(),
        profile: agent_instance.profile.clone(),
    };
    let model = services.model;
    runtime.session = services.session;
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
    runtime.model_config = model.model_config;
    runtime.model_provider = model.model_provider;
    runtime.request_options = model.request_options;
    runtime.provider = model.provider;
    if runtime.fullscreen_stdout_quiet() {
        runtime.push_notice(WorkbenchNotice::info(
            "agent switched",
            &format!(
                "agent={} model={}",
                terminal_inline(&runtime.agent.name),
                terminal_inline(&runtime.model_config.model)
            ),
        ));
    } else if runtime.default_inline_stdout() {
        println!("* Agent");
        println!("  active: {}", terminal_inline(&runtime.agent.name));
        println!("  mode: {}", runtime.agent.mode());
        println!(
            "  workspace: {}",
            terminal_inline(&runtime.workspace.display().to_string())
        );
        println!("  model: {}", terminal_inline(&runtime.model_config.model));
        println!(
            "  permissions: workspace {}, shell {}, network {}",
            runtime.agent.profile.workspace_writes,
            runtime.agent.profile.shell,
            runtime.agent.profile.network
        );
    } else {
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
    }
    Ok(())
}
