// SPDX-License-Identifier: GPL-3.0-only

use crate::{resolve_agent_instance, session_and_registry_for_instance};

use super::{
    interactive::{InteractiveChatRuntime, build_interactive_model_provider},
    workbench::WorkbenchScreenState,
};
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths, ResolvedAgentProfile};
use ikaros_harness::CancellationToken;
use ikaros_models::model_request_options_from_config;
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
) -> Result<(InteractiveChatRuntime, ikaros_harness::SkillRegistry)> {
    let agent_instance = resolve_agent_instance(config, agent_override, workspace, &paths.home)?;
    let agent = ResolvedAgentProfile {
        name: agent_instance.profile_name.clone(),
        profile: agent_instance.profile.clone(),
    };
    let (session, registry) = session_and_registry_for_instance(paths, config, &agent_instance)?;
    let model_config = agent_instance.model_config(&config.model.default).clone();
    let model_provider = agent_instance
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let request_options = model_request_options_from_config(&model_config)?;
    let provider =
        build_interactive_model_provider(&model_config, &model_provider, paths, &session)?;
    Ok((
        InteractiveChatRuntime {
            agent,
            agent_id: agent_instance.agent_id,
            state_dir: agent_instance.state_dir,
            workspace: agent_instance.workspace,
            model_config,
            model_provider,
            provider,
            session,
            chat_session_id,
            request_options,
            pending_inputs: VecDeque::new(),
            pending_content_blocks: Vec::new(),
            screen_state: WorkbenchScreenState::default(),
            persistent_fullscreen: false,
            last_progress: None,
            notices: VecDeque::new(),
            pending_input_drain_requested: false,
        },
        registry,
    ))
}
