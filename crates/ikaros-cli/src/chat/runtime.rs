// SPDX-License-Identifier: GPL-3.0-only

use super::{interactive::InteractiveChatRuntime, workbench::WorkbenchScreenState};
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile};
use ikaros_execution::harness::CancellationToken;
use ikaros_host::chat_runtime_services;
use std::{collections::VecDeque, path::Path};

pub(in crate::chat) fn install_chat_cancellation_signal(cancellation: CancellationToken) {
    tokio::spawn(async move {
        while tokio::signal::ctrl_c().await.is_ok() {
            cancellation.cancel();
            eprintln!("chat_cancel_requested: waiting for the running provider/tool step to stop");
        }
    });
}

pub(in crate::chat) fn initial_interactive_runtime(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
    agent_override: Option<&str>,
    chat_session_id: String,
) -> Result<(
    InteractiveChatRuntime,
    ikaros_execution::harness::SkillRegistry,
)> {
    let services = chat_runtime_services(paths, config, workspace, agent_override)?;
    let agent_instance = services.agent_instance;
    let agent = ResolvedAgentProfile {
        name: agent_instance.profile_name.clone(),
        profile: agent_instance.profile.clone(),
    };
    let model = services.model;
    Ok((
        InteractiveChatRuntime {
            agent,
            agent_id: agent_instance.agent_id,
            state_dir: agent_instance.state_dir,
            workspace: agent_instance.workspace,
            model_config: model.model_config,
            model_provider: model.model_provider,
            provider: model.provider,
            session: services.session,
            chat_session_id,
            request_options: model.request_options,
            pending_inputs: VecDeque::new(),
            pending_content_blocks: Vec::new(),
            screen_state: WorkbenchScreenState::default(),
            default_inline_ui: false,
            last_progress: None,
            notices: VecDeque::new(),
            pending_input_drain_requested: false,
        },
        services.registry,
    ))
}
