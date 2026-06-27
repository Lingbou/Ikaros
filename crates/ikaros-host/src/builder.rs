// SPDX-License-Identifier: GPL-3.0-only

use crate::model_http::EgressModelHttpClient;
use crate::{
    HostAgentContext, HostServices, RuntimeHarness, RuntimeLocation, provider_egress_allowed_hosts,
};
use ikaros_core::{
    AgentInstance, IkarosConfig, IkarosError, IkarosPaths, ModelConfig, RemoteProviderConfig,
    ResolvedAgentProfile, resolve_agent_instance,
};
use ikaros_execution::harness::{
    DockerExecutionEnv, DryRunExecutionEnv, ExecutionEnv, ExecutionSession, GovernedNetworkEgress,
    HttpNetworkEgress, LocalExecutionEnv, NetworkEgressPolicy, NetworkedExecutionEnv,
    SkillRegistry, WorkspaceExecutionEnv,
};
use ikaros_providers::model::{
    ModelProvider, ModelRequestOptions, governed_provider_from_config_with_http_client,
    model_request_options_from_config,
};
use ikaros_skills::{SkillEnvironment, builtin_registry, register_model_backed_skills};
use ikaros_state::memory::LocalMemoryStore;
use ikaros_state::rag::LocalRagStore;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

pub struct ChatModelServices {
    pub model_config: ModelConfig,
    pub model_provider: RemoteProviderConfig,
    pub request_options: ModelRequestOptions,
    pub provider: Box<dyn ModelProvider>,
}

pub struct ChatRuntimeServices {
    pub agent_instance: AgentInstance,
    pub session: ExecutionSession,
    pub registry: SkillRegistry,
    pub model: ChatModelServices,
}

pub struct RuntimeHarnessModelServices {
    pub request_options: ModelRequestOptions,
    pub provider: Box<dyn ModelProvider>,
}

pub struct ApiModelServices {
    pub config: IkarosConfig,
    pub agent_instance: AgentInstance,
    pub model_config: ModelConfig,
    pub provider: Box<dyn ModelProvider>,
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
    let HostAgentContext {
        config,
        agent_instance,
        location,
    } = host_agent_context(paths, workspace, agent_override)?;
    let agent = ResolvedAgentProfile {
        name: agent_instance.profile_name.clone(),
        profile: agent_instance.profile.clone(),
    };
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

pub fn host_agent_context(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> ikaros_core::Result<HostAgentContext> {
    let config = IkarosConfig::load(&paths.config)?;
    host_agent_context_from_config(paths, workspace, agent_override, config)
}

pub fn host_agent_context_shape_checked(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> ikaros_core::Result<HostAgentContext> {
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    host_agent_context_from_config(paths, workspace, agent_override, config)
}

fn host_agent_context_from_config(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    config: IkarosConfig,
) -> ikaros_core::Result<HostAgentContext> {
    let agent_instance = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let location = RuntimeLocation::from_agent_instance(&agent_instance, paths.audit_dir.clone());
    Ok(HostAgentContext {
        config,
        agent_instance,
        location,
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

pub fn chat_runtime_services(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    workspace: &Path,
    agent_override: Option<&str>,
) -> ikaros_core::Result<ChatRuntimeServices> {
    let agent_instance = resolve_agent_instance(config, agent_override, workspace, &paths.home)?;
    chat_runtime_services_for_instance(paths, config, agent_instance)
}

pub fn chat_runtime_services_for_instance(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent_instance: AgentInstance,
) -> ikaros_core::Result<ChatRuntimeServices> {
    let HostServices { session, registry } = services_for_instance(paths, config, &agent_instance)?;
    let model = chat_model_services_for_session(paths, config, &agent_instance, &session)?;
    Ok(ChatRuntimeServices {
        agent_instance,
        session,
        registry,
        model,
    })
}

pub fn chat_model_services_for_session(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent_instance: &AgentInstance,
    session: &ExecutionSession,
) -> ikaros_core::Result<ChatModelServices> {
    let model_config = agent_instance.model_config(&config.model.default).clone();
    let model_provider = agent_instance
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let request_options = model_request_options_from_config(&model_config)?;
    let provider = governed_provider_from_config_with_http_client(
        &model_config,
        &model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(session.env.clone()))),
    )?;
    Ok(ChatModelServices {
        model_config,
        model_provider,
        request_options,
        provider,
    })
}

pub fn runtime_harness_model_services(
    paths: &IkarosPaths,
    harness: &RuntimeHarness,
) -> ikaros_core::Result<RuntimeHarnessModelServices> {
    let model_config = harness
        .agent_instance
        .model_config(&harness.config.model.default);
    let model_provider = harness.agent_instance.effective_model_provider_config(
        &harness.config.model.default,
        &harness.config.providers.model,
    );
    let request_options = model_request_options_from_config(model_config)?;
    let provider = governed_provider_from_config_with_http_client(
        model_config,
        &model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(
            harness.session.env.clone(),
        ))),
    )?;
    Ok(RuntimeHarnessModelServices {
        request_options,
        provider,
    })
}

pub fn runtime_harness_model_provider(
    paths: &IkarosPaths,
    harness: &RuntimeHarness,
) -> ikaros_core::Result<Box<dyn ModelProvider>> {
    Ok(runtime_harness_model_services(paths, harness)?.provider)
}

pub fn chat_model_services_shape_checked(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session: &ExecutionSession,
) -> ikaros_core::Result<ChatModelServices> {
    let HostAgentContext {
        config,
        agent_instance,
        ..
    } = host_agent_context_shape_checked(paths, workspace, agent_override)?;
    chat_model_services_for_session(paths, &config, &agent_instance, session)
}

pub fn api_model_services_shape_checked(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    model_override: Option<&str>,
) -> ikaros_core::Result<ApiModelServices> {
    let HostAgentContext {
        config,
        agent_instance,
        ..
    } = host_agent_context_shape_checked(paths, workspace, agent_override)?;
    let mut model_config = agent_instance.model_config(&config.model.default).clone();
    if let Some(model) = model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        model_config.model = model.to_owned();
    }
    let model_provider = agent_instance
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let HostServices { session, .. } = services_for_instance(paths, &config, &agent_instance)?;
    let provider = governed_provider_from_config_with_http_client(
        &model_config,
        &model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(session.env.clone()))),
    )?;
    Ok(ApiModelServices {
        config,
        agent_instance,
        model_config,
        provider,
    })
}

pub fn session_state_db_candidates(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    include_all_agents: bool,
    existing_only: bool,
) -> ikaros_core::Result<Vec<PathBuf>> {
    let config = IkarosConfig::load(&paths.config)?;
    session_state_db_candidates_with_config(
        paths,
        &config,
        workspace,
        agent_override,
        include_all_agents,
        existing_only,
    )
}

pub fn session_state_db_candidates_with_config(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    workspace: &Path,
    agent_override: Option<&str>,
    include_all_agents: bool,
    existing_only: bool,
) -> ikaros_core::Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    let agent_instance = resolve_agent_instance(config, agent_override, workspace, &paths.home)?;
    push_session_state_db_candidate(
        &mut candidates,
        &mut seen,
        agent_instance.state_dir.join("state.db"),
        existing_only,
    );
    if include_all_agents {
        let agents_dir = paths.home.join("agents");
        if agents_dir.is_dir() {
            for entry in
                fs::read_dir(&agents_dir).map_err(|source| IkarosError::io(&agents_dir, source))?
            {
                let entry = entry.map_err(|source| IkarosError::io(&agents_dir, source))?;
                push_session_state_db_candidate(
                    &mut candidates,
                    &mut seen,
                    entry.path().join("state.db"),
                    existing_only,
                );
            }
        }
    }
    Ok(candidates)
}

fn push_session_state_db_candidate(
    candidates: &mut Vec<PathBuf>,
    seen: &mut BTreeSet<PathBuf>,
    state_db: PathBuf,
    existing_only: bool,
) {
    if (!existing_only || state_db.is_file()) && seen.insert(state_db.clone()) {
        candidates.push(state_db);
    }
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
