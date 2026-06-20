// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{
    AgentInstance, IkarosConfig, IkarosError, IkarosPaths, PolicyDecision, ResolvedAgentProfile,
};
use ikaros_harness::{
    DryRunExecutionEnv, ExecutionEnv, ExecutionSession, GovernedNetworkEgress, HttpNetworkEgress,
    LocalExecutionEnv, NetworkEgressPolicy, NetworkedExecutionEnv, SkillRegistry,
    WorkspaceExecutionEnv,
};
use ikaros_memory::LocalMemoryStore;
use ikaros_rag::LocalRagStore;
use ikaros_skills::{SkillEnvironment, builtin_registry};
use std::{path::Path, sync::Arc, time::Duration};

pub struct RuntimeHarness {
    pub config: IkarosConfig,
    pub agent: ResolvedAgentProfile,
    pub agent_instance: AgentInstance,
    pub session: ExecutionSession,
    pub registry: SkillRegistry,
}

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
    let (session, registry) = session_and_registry_for_instance(paths, &config, &agent_instance)?;
    Ok(RuntimeHarness {
        config,
        agent,
        agent_instance,
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
    let session = ExecutionSession::new_with_agent(workspace, &paths.audit_dir, agent)
        .with_execution_env(runtime_execution_env(config, workspace)?);
    let registry = builtin_registry(skill_environment(paths, workspace, config)?);
    Ok((session, registry))
}

pub fn session_and_registry_for_instance(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent: &AgentInstance,
) -> ikaros_core::Result<(ExecutionSession, SkillRegistry)> {
    let session =
        ExecutionSession::new_with_agent_instance(&agent.workspace, &paths.audit_dir, agent)
            .with_execution_env(runtime_execution_env(config, &agent.workspace)?);
    let registry = builtin_registry(skill_environment(paths, &agent.workspace, config)?);
    Ok((session, registry))
}

pub fn runtime_execution_env(
    config: &IkarosConfig,
    workspace: &Path,
) -> ikaros_core::Result<Arc<dyn ExecutionEnv>> {
    let local_workspace = Arc::new(WorkspaceExecutionEnv::new(
        workspace,
        Arc::new(LocalExecutionEnv),
    )) as Arc<dyn ExecutionEnv>;
    let file_process_env: Arc<dyn ExecutionEnv> = match config
        .execution
        .sandbox
        .backend
        .to_ascii_lowercase()
        .as_str()
    {
        "local" => local_workspace,
        "dry-run" => Arc::new(DryRunExecutionEnv::new(local_workspace)),
        other => {
            return Err(IkarosError::Message(format!(
                "unsupported execution sandbox backend: {other}"
            )));
        }
    };
    let egress = if config.execution.network.enabled {
        let hosts = crate::provider_egress_allowed_hosts(config);
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
        persona_path: paths.persona.clone(),
        skills_dir: paths.skills_dir.clone(),
        voice_tts: config.voice.tts.clone(),
        voice_tts_provider: config.providers.tts.clone(),
        voice_asr: config.voice.asr.clone(),
        voice_asr_provider: config.providers.asr.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn runtime_harness_resolves_agent_instance_identity_and_workspace() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path().join("home"));
        paths.ensure().expect("paths");
        let configured_workspace = temp.path().join("instance-workspace");
        fs::create_dir_all(&configured_workspace).expect("workspace");
        let configured_workspace_yaml =
            serde_json::to_string(&configured_workspace.display().to_string())
                .expect("workspace yaml string");
        fs::write(
            &paths.config,
            format!(
                r#"
agent:
  default: build
  instances:
    repo-build:
      profile: build
      workspace: {configured_workspace_yaml}
"#,
            ),
        )
        .expect("config");

        let fallback_workspace = temp.path().join("fallback-workspace");
        let harness = runtime_harness(&paths, &fallback_workspace, Some("repo-build"))
            .expect("runtime harness");

        assert_eq!(harness.agent_instance.agent_id, "repo-build");
        assert_eq!(harness.agent_instance.profile_name, "build");
        assert_eq!(harness.session.sandbox.workspace_root, configured_workspace);
        let overlay = harness
            .session
            .sandbox
            .agent
            .as_ref()
            .expect("agent overlay");
        assert_eq!(overlay.agent_id.as_deref(), Some("repo-build"));
        assert_eq!(overlay.profile_name, "build");
    }
}
