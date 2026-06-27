// SPDX-License-Identifier: GPL-3.0-only

use crate::{HostServices, RuntimeHarness, RuntimeLocation, provider_egress_allowed_hosts};
use ikaros_core::{
    AgentInstance, IkarosConfig, IkarosError, IkarosPaths, PolicyDecision, ResolvedAgentProfile,
};
use ikaros_harness::{
    DockerExecutionEnv, DryRunExecutionEnv, ExecutionEnv, ExecutionSession, GovernedNetworkEgress,
    HttpNetworkEgress, LocalExecutionEnv, NetworkEgressPolicy, NetworkedExecutionEnv,
    SkillRegistry, WorkspaceExecutionEnv,
};
use ikaros_memory::LocalMemoryStore;
use ikaros_rag::LocalRagStore;
use ikaros_skills::{SkillEnvironment, builtin_registry, register_model_backed_skills};
use std::{path::Path, sync::Arc, time::Duration};

pub fn resolve_agent(
    config: &IkarosConfig,
    agent_override: Option<&str>,
) -> ikaros_core::Result<ResolvedAgentProfile> {
    config.agent.resolve(agent_override).ok_or_else(|| {
        IkarosError::Message(format!(
            "agent profile not found: {}",
            agent_override.unwrap_or(&config.agent.default)
        ))
    })
}

pub fn session_and_registry(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> ikaros_core::Result<(ExecutionSession, SkillRegistry)> {
    let harness = runtime_harness(paths, workspace, agent_override)?;
    Ok((harness.session, harness.registry))
}

pub fn runtime_harness(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> ikaros_core::Result<RuntimeHarness> {
    let config = IkarosConfig::load(&paths.config)?;
    let agent_instance = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let agent = ResolvedAgentProfile {
        name: agent_instance.profile_name.clone(),
        profile: agent_instance.profile.clone(),
    };
    let location = RuntimeLocation::from_agent_instance(&agent_instance, paths.audit_dir.clone());
    let HostServices { session, registry } =
        services_for_instance(paths, &config, &agent_instance)?;
    Ok(RuntimeHarness {
        config,
        agent,
        agent_instance,
        location,
        session,
        registry,
    })
}

pub fn resolve_agent_instance(
    config: &IkarosConfig,
    agent_override: Option<&str>,
    workspace: &Path,
    state_root: &Path,
) -> ikaros_core::Result<AgentInstance> {
    config
        .agent
        .resolve_instance(agent_override, workspace, state_root)
        .ok_or_else(|| {
            IkarosError::Message(format!(
                "agent profile or instance not found: {}",
                agent_override.unwrap_or(&config.agent.default)
            ))
        })
}

pub fn session_and_registry_for_agent(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
    agent: &ResolvedAgentProfile,
) -> ikaros_core::Result<(ExecutionSession, SkillRegistry)> {
    let HostServices { session, registry } = services_for_agent(paths, workspace, config, agent)?;
    Ok((session, registry))
}

pub fn session_and_registry_for_instance(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent: &AgentInstance,
) -> ikaros_core::Result<(ExecutionSession, SkillRegistry)> {
    let HostServices { session, registry } = services_for_instance(paths, config, agent)?;
    Ok((session, registry))
}

pub fn services_for_agent(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
    agent: &ResolvedAgentProfile,
) -> ikaros_core::Result<HostServices> {
    let session = ExecutionSession::new_with_agent(workspace, &paths.audit_dir, agent)
        .with_execution_env(runtime_execution_env(config, workspace)?);
    let mut registry = builtin_registry(skill_environment(paths, workspace, config)?);
    register_model_backed_skills(
        &mut registry,
        config.model.default.clone(),
        config.effective_model_provider(),
    );
    Ok(HostServices { session, registry })
}

pub fn services_for_instance(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent: &AgentInstance,
) -> ikaros_core::Result<HostServices> {
    let session =
        ExecutionSession::new_with_agent_instance(&agent.workspace, &paths.audit_dir, agent)
            .with_execution_env(runtime_execution_env(config, &agent.workspace)?);
    let mut registry = builtin_registry(skill_environment(paths, &agent.workspace, config)?);
    register_model_backed_skills(
        &mut registry,
        agent.model_config(&config.model.default).clone(),
        agent.effective_model_provider_config(&config.model.default, &config.providers.model),
    );
    Ok(HostServices { session, registry })
}

pub fn runtime_execution_env(
    config: &IkarosConfig,
    workspace: &Path,
) -> ikaros_core::Result<Arc<dyn ExecutionEnv>> {
    let local_workspace = || {
        Arc::new(WorkspaceExecutionEnv::new(
            workspace,
            Arc::new(LocalExecutionEnv),
        )) as Arc<dyn ExecutionEnv>
    };
    let file_process_env: Arc<dyn ExecutionEnv> = match config
        .execution
        .sandbox
        .backend
        .to_ascii_lowercase()
        .as_str()
    {
        "local" => local_workspace(),
        "dry-run" => Arc::new(DryRunExecutionEnv::new(local_workspace())),
        "docker" => Arc::new(WorkspaceExecutionEnv::new(
            workspace,
            Arc::new(DockerExecutionEnv::new(
                workspace,
                &config.execution.sandbox.image,
            )),
        )),
        other => {
            return Err(IkarosError::Message(format!(
                "unsupported execution sandbox backend: {other}"
            )));
        }
    };
    let egress = if config.execution.network.enabled {
        let hosts = provider_egress_allowed_hosts(config);
        let policy = NetworkEgressPolicy::allow_hosts(hosts);
        Arc::new(GovernedNetworkEgress::new(
            policy,
            Arc::new(HttpNetworkEgress::new(Duration::from_millis(
                config.execution.network.timeout_ms,
            ))?),
        ))
    } else {
        Arc::new(GovernedNetworkEgress::deny_by_default())
    };
    Ok(Arc::new(NetworkedExecutionEnv::new(
        file_process_env,
        egress,
    )))
}

pub fn skill_environment(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
) -> ikaros_core::Result<SkillEnvironment> {
    Ok(SkillEnvironment {
        workspace_root: workspace.to_path_buf(),
        memory_store: LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)?,
        rag_index: LocalRagStore::new(&paths.rag_dir, &config.rag.backend)?,
        rag_config: config.rag.clone(),
        rag_provider: config.providers.embedding.clone(),
        persona_path: paths.persona_dir.clone(),
        skills_dir: paths.skills_dir.clone(),
        voice_tts: config.voice.tts.clone(),
        voice_tts_provider: config.providers.tts.clone(),
        voice_asr: config.voice.asr.clone(),
        voice_asr_provider: config.providers.asr.clone(),
        web_search_provider: config.providers.search.clone(),
        coding_session: None,
    })
}

pub fn recent_policy_decisions(
    session: &ExecutionSession,
) -> ikaros_core::Result<Vec<PolicyDecision>> {
    let decisions = session
        .audit
        .read_all()?
        .into_iter()
        .filter_map(|event| event.decision)
        .collect::<Vec<_>>();
    let start = decisions.len().saturating_sub(12);
    Ok(decisions[start..].to_vec())
}
