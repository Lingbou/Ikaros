// SPDX-License-Identifier: GPL-3.0-only
//! Core domain types shared by the Ikaros runtime.

mod agent;
mod config;
mod context;
mod error;
mod paths;
mod redaction;
mod secret;
mod task;
mod time;

pub use agent::{
    AgentAuthScope, AgentConfig, AgentHistoryScope, AgentInstance, AgentInstanceConfig, AgentMode,
    AgentPermission, AgentProfile, AgentRouteBinding, AgentSessionPolicy, ResolvedAgentProfile,
};
pub use config::{
    ConfigValidationIssue, ConfigValidationReport, ExternalMemoryProviderConfig,
    ExternalProvidersConfig, IkarosConfig, LocalStoreConfig, MemoryConfig, ModelConfig,
    ModelParamsConfig, ModelReasoningConfig, ModelTable, PolicyConfig, RagConfig,
    RemoteProviderConfig, SelfModifyCheckProfileConfig, SelfModifyConfig, VoiceConfig,
    VoiceProviderConfig,
};
pub use context::{ContextBuilder, RuntimeContext};
pub use error::{IkarosError, Result};
pub use paths::IkarosPaths;
pub use redaction::{contains_secret_like, redact_json, redact_secrets, reject_secret_like};
pub use secret::{resolve_config_secret, resolve_config_value};
pub use task::{
    Plan, PlanStep, PolicyDecision, RiskLevel, RuntimeCoordinator, RuntimeEvent, Task,
    TaskRunnerReport, TaskState, ToolCall, ToolResult,
};
pub use time::now_rfc3339;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_to_openai_compatible_provider_shape() {
        let config = IkarosConfig::default();
        let agent = config.agent.active();
        assert_eq!(agent.name, "build");
        assert_eq!(agent.mode(), &AgentMode::Build);
        assert!(config.agent.profiles.contains_key("plan"));
        assert_eq!(config.model.default.provider, "openai-compatible");
        assert_eq!(config.policy.workspace_writes, "ask");
        assert_eq!(config.rag.backend, "jsonl");
        assert!(config.memory.external_providers.is_empty());
        assert_eq!(config.model.default.runtime, "harness-agent-loop");
        assert_eq!(
            config.model.default.transport,
            "openai-compatible-chat-completions"
        );
        assert!(config.providers.model.base_url.is_empty());
        assert!(config.providers.model.api_key.is_empty());
        assert!(config.model.default.model.is_empty());
        assert_eq!(config.rag.embedding_provider, "openai-compatible");
        assert!(config.providers.embedding.base_url.is_empty());
        assert!(config.providers.embedding.api_key.is_empty());
        assert!(config.rag.embedding_model.is_empty());
        assert_eq!(config.voice.tts.provider, "openai-compatible");
        assert_eq!(config.voice.asr.provider, "openai-compatible");
        assert!(config.self_modify.check_profiles.is_empty());
    }

    #[test]
    fn config_parses_agent_instances_separate_from_profiles() {
        let config: IkarosConfig = yaml_serde::from_str(
            r#"
agent:
  default: build
  profiles:
    research:
      mode: general
      description: "Repository research"
      persona_overlay: "Stay read-heavy."
      memory_context: true
      rag_context: true
      workspace_writes: deny
      shell: ask
      network: deny
  instances:
    repo-research:
      profile: research
      workspace: /workspace/repo
      state_dir: /state/repo-research
      session_policy:
        history_scope: agent
        allow_session_switch: false
        max_parallel_subagents: 2
      auth_scope:
        local_only: true
        allow_network: deny
      route_bindings:
        - channel: cli
          thread: main
"#,
        )
        .expect("config");

        let instance = config
            .agent
            .resolve_instance(
                Some("repo-research"),
                "/fallback-workspace",
                "/fallback-state",
            )
            .expect("instance");
        assert_eq!(instance.agent_id, "repo-research");
        assert_eq!(instance.profile_name, "research");
        assert_eq!(instance.profile.workspace_writes, AgentPermission::Deny);
        assert_eq!(
            instance.workspace,
            std::path::PathBuf::from("/workspace/repo")
        );
        assert_eq!(
            instance.state_dir,
            std::path::PathBuf::from("/state/repo-research")
        );
        assert_eq!(
            instance.session_policy.history_scope,
            AgentHistoryScope::Agent
        );
        assert_eq!(instance.session_policy.max_parallel_subagents, 2);
        assert_eq!(instance.auth_scope.allow_network, AgentPermission::Deny);
        assert_eq!(instance.route_bindings[0].channel, "cli");
    }

    #[test]
    fn config_parses_external_memory_providers() {
        let config: IkarosConfig = yaml_serde::from_str(
            r#"
memory:
  backend: sqlite
  external_providers:
    - id: remote-a
      provider: plugin
      enabled: true
      endpoint: http://127.0.0.1:8787
      api_key: memory-provider-key
"#,
        )
        .expect("config");

        assert_eq!(config.memory.backend, "sqlite");
        assert_eq!(config.memory.external_providers.len(), 1);
        let provider = &config.memory.external_providers[0];
        assert_eq!(provider.id, "remote-a");
        assert_eq!(provider.provider, "plugin");
        assert!(provider.enabled);
        assert_eq!(provider.api_key.as_deref(), Some("memory-provider-key"));
    }

    #[test]
    fn config_parses_agent_profiles() {
        let config: IkarosConfig = yaml_serde::from_str(
            r#"
agent:
  default: research
  profiles:
    research:
      mode: general
      description: "Repository research"
      persona_overlay: "Stay read-heavy and cite local context."
      memory_context: false
      rag_context: true
      workspace_writes: deny
      shell: ask
      network: deny
"#,
        )
        .expect("config");
        let agent = config
            .agent
            .resolve(Some("research"))
            .expect("research profile");
        assert_eq!(agent.name, "research");
        assert_eq!(agent.mode(), &AgentMode::General);
        assert!(!agent.profile.memory_context);
        assert_eq!(agent.profile.network, AgentPermission::Deny);
    }

    #[test]
    fn config_parses_self_modify_check_profiles() {
        let config: IkarosConfig = yaml_serde::from_str(
            r#"
self_modify:
  check_profiles:
    runtime_patch:
      commands:
        - cargo check --workspace --all-features
      reason: "Runtime changes must compile."
"#,
        )
        .expect("config");

        let profile = config
            .self_modify
            .check_profiles
            .get("runtime_patch")
            .expect("profile");
        assert_eq!(
            profile.commands,
            vec!["cargo check --workspace --all-features"]
        );
        assert_eq!(
            profile.reason.as_deref(),
            Some("Runtime changes must compile.")
        );
    }

    #[test]
    fn paths_respect_custom_home() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path());
        assert_eq!(paths.config, temp.path().join("config.yaml"));
        assert_eq!(paths.rag_dir, temp.path().join("rag"));
        assert_eq!(paths.automation_dir, temp.path().join("automation"));
        assert_eq!(paths.gateway_dir, temp.path().join("gateway"));
    }

    #[test]
    fn redacts_secret_like_tokens() {
        let redacted = redact_secrets("Authorization sk-test-secret api_key=abc");
        assert!(!redacted.contains("sk-test-secret"));
        assert!(!redacted.contains("abc"));
        assert!(redacted.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn redacts_secret_like_json_keys() {
        let redacted = redact_json(serde_json::json!({
            "token": "abc123",
            "nested": {"github_token": "ghp_test"},
            "safe": "visible",
        }));
        let raw = serde_json::to_string(&redacted).expect("json");
        assert!(!raw.contains("abc123"));
        assert!(!raw.contains("ghp_test"));
        assert!(raw.contains("visible"));
        assert!(raw.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn rejects_secret_like_memory() {
        let err = reject_secret_like("password=hunter2", "memory content").expect_err("rejected");
        assert!(err.to_string().contains("memory content"));
    }
}
