// SPDX-License-Identifier: GPL-3.0-only

use crate::{AgentConfig, IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct IkarosConfig {
    pub providers: ExternalProvidersConfig,
    pub agent: AgentConfig,
    pub model: ModelTable,
    pub policy: PolicyConfig,
    pub memory: MemoryConfig,
    pub chat_history: LocalStoreConfig,
    pub rag: RagConfig,
    pub voice: VoiceConfig,
    pub self_modify: SelfModifyConfig,
}

impl IkarosConfig {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Err(IkarosError::Message(format!(
                "config file not found: {}; run `ikaros init` to create config.yaml under IKAROS_HOME",
                path.display()
            )));
        }
        let raw = fs::read_to_string(path).map_err(|source| IkarosError::io(path, source))?;
        Ok(yaml_serde::from_str(&raw)?)
    }

    pub fn write_default_config(path: &Path) -> Result<()> {
        let raw = r#"# Ikaros local runtime configuration.
# This file belongs under IKAROS_HOME and may contain plaintext local credentials.

providers:
  model:
    # API key for the default chat/completion provider.
    api_key: ""
    # Base URL for the default chat/completion provider.
    # Example: https://api.example.com/v1.
    base_url: ""
  embedding:
    # API key for the remote embedding provider.
    api_key: ""
    # Base URL for the remote embedding provider.
    # Use the same provider base URL as embeddings support, or a separate embedding service URL.
    base_url: ""
  tts:
    # API key for the remote text-to-speech provider.
    api_key: ""
    # Base URL for the remote text-to-speech provider.
    # Use the same provider base URL as TTS support, or a separate speech service URL.
    base_url: ""
  asr:
    # API key for the remote speech-to-text provider.
    api_key: ""
    # Base URL for the remote speech-to-text provider.
    # Use the same provider base URL as ASR support, or a separate speech service URL.
    base_url: ""

agent:
  # Default agent profile used when no agent or instance is selected explicitly.
  default: build
  profiles:
    build:
      # Runtime mode for ordinary implementation work.
      mode: build
      # Human-readable description shown in diagnostics.
      description: "Default implementation mode for ordinary local development work."
      # Prompt overlay for this profile; it must not bypass policy gates.
      persona_overlay: "Operate as the default local implementation agent. Use harnessed tools and keep writes approval-aware."
      # Include local memory context in turns started by this profile.
      memory_context: true
      # Include local RAG context in turns started by this profile.
      rag_context: true
      # Workspace write policy: allow, ask, or deny.
      workspace_writes: ask
      # Shell policy: allow, ask, or deny.
      shell: allow
      # Network policy: allow, ask, or deny.
      network: ask
    plan:
      # Runtime mode for read-only planning.
      mode: plan
      # Human-readable description shown in diagnostics.
      description: "Read-only planning and code exploration mode."
      # Prompt overlay for planning turns.
      persona_overlay: "Operate in read-only planning mode. Prefer analysis, design notes, and explicit implementation plans; do not request file edits."
      # Include local memory context in planning turns.
      memory_context: true
      # Include local RAG context in planning turns.
      rag_context: true
      # Planning should not write to the workspace by default.
      workspace_writes: deny
      # Shell policy for planning.
      shell: ask
      # Network policy for planning.
      network: ask
    general:
      # Runtime mode for general local research.
      mode: general
      # Human-readable description shown in diagnostics.
      description: "General research mode for multi-step local questions."
      # Prompt overlay for general research turns.
      persona_overlay: "Operate as a general-purpose research agent. Gather local context first and keep recommendations grounded in available evidence."
      # Include local memory context in general turns.
      memory_context: true
      # Include local RAG context in general turns.
      rag_context: true
      # Workspace write policy for general turns.
      workspace_writes: ask
      # Shell policy for general turns.
      shell: ask
      # Network policy for general turns.
      network: ask
  # Agent instances are identities with their own workspace/state/session/auth/routing.
  # Profiles remain persona and policy overlays.
  # instances:
  #   local-build:
  #     profile: build
  #     workspace: /path/to/workspace
  #     state_dir: /path/to/.ikaros/agents/local-build
  #     session_policy:
  #       history_scope: workspace # agent, session, workspace
  #       allow_session_switch: true
  #       max_parallel_subagents: 4
  #     auth_scope:
  #       local_only: true
  #       allow_network: ask
  #     route_bindings:
  #       - channel: cli

model:
  default:
    # Provider family: openai-compatible, anthropic, ollama, or mock for tests only.
    provider: openai-compatible
    # Agent runtime implementation that owns the turn loop.
    runtime: harness-agent-loop
    # Wire protocol used by the provider adapter.
    transport: openai-compatible-chat-completions
    # Model identifier sent to the provider.
    model: ""
    # Request timeout in milliseconds.
    timeout_ms: 30000
    # Provider retry count after the first failed attempt.
    max_retries: 0
    # Optional per-minute request budget for model calls.
    rate_limit_per_minute: 60
    # Optional daily token budget recorded by the usage ledger.
    daily_token_budget: 100000

policy:
  # Default workspace write policy used when a profile does not override it.
  workspace_writes: ask
  # Default network policy used when a profile does not override it.
  network: ask
  # Redact secret-like values from audit records.
  audit_redaction: true

memory:
  # Local memory backend: jsonl or sqlite.
  backend: jsonl
  # Only one external memory provider may be enabled at a time.
  # external_providers:
  #   - id: team-memory
  #     provider: plugin
  #     enabled: false
  #     endpoint: http://127.0.0.1:8787
  #     api_key: ""

chat_history:
  # Local chat history backend: jsonl or sqlite.
  backend: jsonl

rag:
  # Local RAG index backend: jsonl or sqlite.
  backend: jsonl
  # Embedding provider: hash, sparse, mock, or openai-compatible.
  embedding_provider: openai-compatible
  # Embedding model name sent to the provider.
  embedding_model: ""
  # Embedding request timeout in milliseconds.
  embedding_timeout_ms: 30000
  # Provider retry count for embedding calls.
  embedding_max_retries: 0

voice:
  tts:
    # Text-to-speech provider: mock or openai-compatible.
    provider: openai-compatible
    # TTS model name sent to the provider.
    model: ""
    # TTS request timeout in milliseconds.
    timeout_ms: 30000
    # Provider retry count for TTS calls.
    max_retries: 0
    # Default TTS voice name.
    voice: default
  asr:
    # Speech-to-text provider: mock or openai-compatible.
    provider: openai-compatible
    # ASR model name sent to the provider.
    model: ""
    # ASR request timeout in milliseconds.
    timeout_ms: 30000
    # Provider retry count for ASR calls.
    max_retries: 0

# Optional self-modify check profiles override built-in checks by change kind.
# Commands are still validated against the restricted test/check/lint/build command set.
# self_modify:
#   check_profiles:
#     runtime_patch:
#       commands:
#         - cargo check --workspace --all-features
#       reason: "Runtime changes must keep the workspace compiling."
"#;
        fs::write(path, raw).map_err(|source| IkarosError::io(path, source))
    }

    pub fn validate_file(path: &Path) -> Result<ConfigValidationReport> {
        if !path.exists() {
            let mut report = ConfigValidationReport::default();
            report.error(
                "config",
                format!(
                    "{} does not exist; run `ikaros init` or create config.yaml under IKAROS_HOME",
                    path.display()
                ),
            );
            return Ok(report);
        }
        let raw = fs::read_to_string(path).map_err(|source| IkarosError::io(path, source))?;
        Self::validate_yaml(&raw)
    }

    pub fn validate_yaml(raw: &str) -> Result<ConfigValidationReport> {
        let value: yaml_serde::Value = yaml_serde::from_str(raw)?;
        let mut report = ConfigValidationReport::default();
        validate_yaml_shape(&value, &mut report);

        let config: Self = yaml_serde::from_str(raw)?;
        config.validate_into(&mut report);
        Ok(report)
    }

    pub fn validate(&self) -> ConfigValidationReport {
        let mut report = ConfigValidationReport::default();
        self.validate_into(&mut report);
        report
    }

    fn validate_into(&self, report: &mut ConfigValidationReport) {
        validate_agent_config(&self.agent, report);
        validate_model_config(&self.model.default, &self.providers.model, report);
        validate_policy_config(&self.policy, report);
        validate_memory_config(&self.memory, report);
        validate_local_backend("chat_history.backend", &self.chat_history.backend, report);
        validate_rag_config(&self.rag, &self.providers.embedding, report);
        validate_voice_config(
            "voice.tts",
            &self.voice.tts,
            &self.providers.tts,
            true,
            report,
        );
        validate_voice_config(
            "voice.asr",
            &self.voice.asr,
            &self.providers.asr,
            false,
            report,
        );
        validate_self_modify_config(&self.self_modify, report);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigValidationReport {
    pub errors: Vec<ConfigValidationIssue>,
    pub warnings: Vec<ConfigValidationIssue>,
}

impl ConfigValidationReport {
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }

    fn error(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.errors.push(ConfigValidationIssue {
            path: path.into(),
            message: message.into(),
        });
    }

    fn warning(&mut self, path: impl Into<String>, message: impl Into<String>) {
        self.warnings.push(ConfigValidationIssue {
            path: path.into(),
            message: message.into(),
        });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigValidationIssue {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExternalProvidersConfig {
    pub model: RemoteProviderConfig,
    pub embedding: RemoteProviderConfig,
    pub tts: RemoteProviderConfig,
    pub asr: RemoteProviderConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RemoteProviderConfig {
    pub api_key: String,
    pub base_url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_provider_settings_are_schema_only() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        fs::write(
            &path,
            r#"
providers:
  model:
    api_key: model-secret
    base_url: https://model.example/v1
  embedding:
    api_key: rag-secret
    base_url: https://embedding.example/v1
  tts:
    api_key: tts-secret
    base_url: https://tts.example/v1
  asr:
    api_key: asr-secret
    base_url: https://asr.example/v1
"#,
        )
        .expect("write");

        let config = IkarosConfig::load(&path).expect("config");
        assert_eq!(config.providers.model.api_key, "model-secret");
        assert_eq!(config.providers.model.base_url, "https://model.example/v1");
        assert_eq!(config.providers.embedding.api_key, "rag-secret");
        assert_eq!(
            config.providers.embedding.base_url,
            "https://embedding.example/v1"
        );
        assert_eq!(config.providers.tts.api_key, "tts-secret");
        assert_eq!(config.providers.tts.base_url, "https://tts.example/v1");
        assert_eq!(config.providers.asr.api_key, "asr-secret");
        assert_eq!(config.providers.asr.base_url, "https://asr.example/v1");
    }

    #[test]
    fn generated_default_config_parses() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        IkarosConfig::write_default_config(&path).expect("write");

        let config = IkarosConfig::load(&path).expect("load generated config");
        assert_eq!(config.model.default.provider, "openai-compatible");
        assert!(config.providers.model.api_key.is_empty());
        assert!(config.providers.model.base_url.is_empty());
        assert_eq!(config.rag.embedding_provider, "openai-compatible");
        assert!(config.providers.embedding.base_url.is_empty());
        assert_eq!(config.voice.tts.provider, "openai-compatible");
        assert!(config.providers.tts.base_url.is_empty());
    }

    #[test]
    fn config_validation_accepts_complete_runtime_config() {
        let report = IkarosConfig::validate_yaml(
            r#"
providers:
  model:
    api_key: model-secret
    base_url: https://api.example/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: example-chat
    timeout_ms: 30000

rag:
  backend: sqlite
  embedding_provider: hash

voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr
"#,
        )
        .expect("validate");

        assert!(report.is_valid(), "{report:#?}");
    }

    #[test]
    fn config_validation_allows_ollama_default_base_url() {
        let report = IkarosConfig::validate_yaml(
            r#"
model:
  default:
    provider: ollama
    runtime: harness-agent-loop
    transport: ollama-chat
    model: llama3.2

rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
  asr:
    provider: mock
"#,
        )
        .expect("validate");

        assert!(report.is_valid(), "{report:#?}");
    }

    #[test]
    fn config_validation_rejects_unknown_fields_and_missing_remote_settings() {
        let report = IkarosConfig::validate_yaml(
            r#"
model:
  default:
    provider: openai-compatible
    transport: openai-compatible-chat-completions
    extra_field: true
"#,
        )
        .expect("validate");

        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "model.default.extra_field")
        );
        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "providers.model.base_url")
        );
        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "providers.model.api_key")
        );
    }

    #[test]
    fn config_validation_rejects_enabled_external_memory_provider() {
        let report = IkarosConfig::validate_yaml(
            r#"
providers:
  model:
    api_key: model-secret
    base_url: https://api.example/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: example-chat

rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
  asr:
    provider: mock

memory:
  external_providers:
    - id: remote
      provider: plugin
      enabled: true
      endpoint: http://127.0.0.1:8787
"#,
        )
        .expect("validate");

        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "memory.external_providers[0].enabled")
        );
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ModelTable {
    pub default: ModelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ModelConfig {
    pub provider: String,
    pub runtime: String,
    pub transport: String,
    pub model: String,
    pub timeout_ms: u64,
    pub max_retries: u8,
    pub rate_limit_per_minute: Option<u32>,
    pub daily_token_budget: Option<u32>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: "openai-compatible".into(),
            runtime: "harness-agent-loop".into(),
            transport: "openai-compatible-chat-completions".into(),
            model: String::new(),
            timeout_ms: 30_000,
            max_retries: 0,
            rate_limit_per_minute: None,
            daily_token_budget: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct PolicyConfig {
    pub workspace_writes: String,
    pub network: String,
    pub audit_redaction: bool,
}

impl Default for PolicyConfig {
    fn default() -> Self {
        Self {
            workspace_writes: "ask".into(),
            network: "ask".into(),
            audit_redaction: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct MemoryConfig {
    pub backend: String,
    pub external_providers: Vec<ExternalMemoryProviderConfig>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            backend: "jsonl".into(),
            external_providers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ExternalMemoryProviderConfig {
    pub id: String,
    pub provider: String,
    pub enabled: bool,
    pub endpoint: Option<String>,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct LocalStoreConfig {
    pub backend: String,
}

impl Default for LocalStoreConfig {
    fn default() -> Self {
        Self {
            backend: "jsonl".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct RagConfig {
    pub backend: String,
    pub embedding_provider: String,
    pub embedding_model: String,
    pub embedding_timeout_ms: u64,
    pub embedding_max_retries: u8,
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            backend: "jsonl".into(),
            embedding_provider: "openai-compatible".into(),
            embedding_model: String::new(),
            embedding_timeout_ms: 30_000,
            embedding_max_retries: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VoiceConfig {
    pub tts: VoiceProviderConfig,
    pub asr: VoiceProviderConfig,
}

impl Default for VoiceConfig {
    fn default() -> Self {
        Self {
            tts: VoiceProviderConfig::remote_tts(),
            asr: VoiceProviderConfig::remote_asr(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct VoiceProviderConfig {
    pub provider: String,
    pub model: String,
    pub timeout_ms: u64,
    pub max_retries: u8,
    pub voice: Option<String>,
}

impl VoiceProviderConfig {
    pub fn remote_tts() -> Self {
        Self {
            provider: "openai-compatible".into(),
            model: String::new(),
            timeout_ms: 30_000,
            max_retries: 0,
            voice: Some("default".into()),
        }
    }

    pub fn remote_asr() -> Self {
        Self {
            provider: "openai-compatible".into(),
            model: String::new(),
            timeout_ms: 30_000,
            max_retries: 0,
            voice: None,
        }
    }

    pub fn mock_tts() -> Self {
        Self {
            provider: "mock".into(),
            model: "mock-tts".into(),
            timeout_ms: 30_000,
            max_retries: 0,
            voice: Some("default".into()),
        }
    }

    pub fn mock_asr() -> Self {
        Self {
            provider: "mock".into(),
            model: "mock-asr".into(),
            timeout_ms: 30_000,
            max_retries: 0,
            voice: None,
        }
    }
}

impl Default for VoiceProviderConfig {
    fn default() -> Self {
        Self::remote_tts()
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SelfModifyConfig {
    pub check_profiles: BTreeMap<String, SelfModifyCheckProfileConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SelfModifyCheckProfileConfig {
    pub commands: Vec<String>,
    pub reason: Option<String>,
}

fn validate_yaml_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(root) = expect_mapping(value, "config", report) else {
        return;
    };
    check_allowed_fields(
        root,
        "config",
        &[
            "providers",
            "agent",
            "model",
            "policy",
            "memory",
            "chat_history",
            "rag",
            "voice",
            "self_modify",
        ],
        report,
    );
    if let Some(providers) = mapping_get(root, "providers") {
        validate_providers_shape(providers, report);
    }
    if let Some(agent) = mapping_get(root, "agent") {
        validate_agent_shape(agent, report);
    }
    if let Some(model) = mapping_get(root, "model") {
        validate_model_shape(model, report);
    }
    if let Some(policy) = mapping_get(root, "policy") {
        check_mapping_shape(
            policy,
            "policy",
            &["workspace_writes", "network", "audit_redaction"],
            report,
        );
    }
    if let Some(memory) = mapping_get(root, "memory") {
        validate_memory_shape(memory, report);
    }
    if let Some(chat_history) = mapping_get(root, "chat_history") {
        check_mapping_shape(chat_history, "chat_history", &["backend"], report);
    }
    if let Some(rag) = mapping_get(root, "rag") {
        check_mapping_shape(
            rag,
            "rag",
            &[
                "backend",
                "embedding_provider",
                "embedding_model",
                "embedding_timeout_ms",
                "embedding_max_retries",
            ],
            report,
        );
    }
    if let Some(voice) = mapping_get(root, "voice") {
        validate_voice_shape(voice, report);
    }
    if let Some(self_modify) = mapping_get(root, "self_modify") {
        validate_self_modify_shape(self_modify, report);
    }
}

fn validate_providers_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(
        value,
        "providers",
        &["model", "embedding", "tts", "asr"],
        report,
    ) else {
        return;
    };
    for key in ["model", "embedding", "tts", "asr"] {
        if let Some(provider) = mapping_get(map, key) {
            check_mapping_shape(
                provider,
                format!("providers.{key}"),
                &["api_key", "base_url"],
                report,
            );
        }
    }
}

fn validate_agent_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(
        value,
        "agent",
        &["default", "profiles", "instances"],
        report,
    ) else {
        return;
    };
    if let Some(profiles) = mapping_get(map, "profiles") {
        validate_dynamic_mapping_shape(
            profiles,
            "agent.profiles",
            &[
                "mode",
                "description",
                "persona_overlay",
                "memory_context",
                "rag_context",
                "workspace_writes",
                "shell",
                "network",
            ],
            report,
        );
    }
    if let Some(instances) = mapping_get(map, "instances") {
        let Some(instance_map) = expect_mapping(instances, "agent.instances", report) else {
            return;
        };
        check_string_keys(instance_map, "agent.instances", report);
        for (name, instance) in string_entries(instance_map) {
            let path = format!("agent.instances.{name}");
            let Some(map) = check_mapping_shape(
                instance,
                &path,
                &[
                    "profile",
                    "workspace",
                    "state_dir",
                    "session_policy",
                    "auth_scope",
                    "route_bindings",
                ],
                report,
            ) else {
                continue;
            };
            if let Some(policy) = mapping_get(map, "session_policy") {
                check_mapping_shape(
                    policy,
                    format!("{path}.session_policy"),
                    &[
                        "history_scope",
                        "allow_session_switch",
                        "max_parallel_subagents",
                    ],
                    report,
                );
            }
            if let Some(auth_scope) = mapping_get(map, "auth_scope") {
                check_mapping_shape(
                    auth_scope,
                    format!("{path}.auth_scope"),
                    &["local_only", "allow_network"],
                    report,
                );
            }
            if let Some(bindings) = mapping_get(map, "route_bindings") {
                validate_sequence_mapping_shape(
                    bindings,
                    format!("{path}.route_bindings"),
                    &["channel", "account", "peer", "thread"],
                    report,
                );
            }
        }
    }
}

fn validate_model_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(value, "model", &["default"], report) else {
        return;
    };
    if let Some(default) = mapping_get(map, "default") {
        check_mapping_shape(
            default,
            "model.default",
            &[
                "provider",
                "runtime",
                "transport",
                "model",
                "timeout_ms",
                "max_retries",
                "rate_limit_per_minute",
                "daily_token_budget",
            ],
            report,
        );
    }
}

fn validate_memory_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) =
        check_mapping_shape(value, "memory", &["backend", "external_providers"], report)
    else {
        return;
    };
    if let Some(external) = mapping_get(map, "external_providers") {
        validate_sequence_mapping_shape(
            external,
            "memory.external_providers",
            &["id", "provider", "enabled", "endpoint", "api_key"],
            report,
        );
    }
}

fn validate_voice_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(value, "voice", &["tts", "asr"], report) else {
        return;
    };
    for key in ["tts", "asr"] {
        if let Some(provider) = mapping_get(map, key) {
            check_mapping_shape(
                provider,
                format!("voice.{key}"),
                &["provider", "model", "timeout_ms", "max_retries", "voice"],
                report,
            );
        }
    }
}

fn validate_self_modify_shape(value: &yaml_serde::Value, report: &mut ConfigValidationReport) {
    let Some(map) = check_mapping_shape(value, "self_modify", &["check_profiles"], report) else {
        return;
    };
    if let Some(check_profiles) = mapping_get(map, "check_profiles") {
        validate_dynamic_mapping_shape(
            check_profiles,
            "self_modify.check_profiles",
            &["commands", "reason"],
            report,
        );
    }
}

fn check_mapping_shape<'a>(
    value: &'a yaml_serde::Value,
    path: impl AsRef<str>,
    allowed: &[&str],
    report: &mut ConfigValidationReport,
) -> Option<&'a yaml_serde::Mapping> {
    let path = path.as_ref();
    let map = expect_mapping(value, path, report)?;
    check_allowed_fields(map, path, allowed, report);
    Some(map)
}

fn validate_dynamic_mapping_shape(
    value: &yaml_serde::Value,
    path: impl AsRef<str>,
    allowed: &[&str],
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    let Some(map) = expect_mapping(value, path, report) else {
        return;
    };
    check_string_keys(map, path, report);
    for (key, value) in string_entries(map) {
        check_mapping_shape(value, format!("{path}.{key}"), allowed, report);
    }
}

fn validate_sequence_mapping_shape(
    value: &yaml_serde::Value,
    path: impl AsRef<str>,
    allowed: &[&str],
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    let yaml_serde::Value::Sequence(items) = value else {
        report.error(path, "must be a YAML sequence");
        return;
    };
    for (index, item) in items.iter().enumerate() {
        check_mapping_shape(item, format!("{path}[{index}]"), allowed, report);
    }
}

fn expect_mapping<'a>(
    value: &'a yaml_serde::Value,
    path: impl AsRef<str>,
    report: &mut ConfigValidationReport,
) -> Option<&'a yaml_serde::Mapping> {
    match value {
        yaml_serde::Value::Mapping(map) => Some(map),
        _ => {
            report.error(path.as_ref(), "must be a YAML mapping");
            None
        }
    }
}

fn check_allowed_fields(
    map: &yaml_serde::Mapping,
    path: impl AsRef<str>,
    allowed: &[&str],
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    let allowed = allowed.iter().copied().collect::<BTreeSet<_>>();
    for key in map.keys() {
        let Some(key) = key.as_str() else {
            report.error(path, "mapping keys must be strings");
            continue;
        };
        if !allowed.contains(key) {
            report.error(format!("{path}.{key}"), "unknown configuration field");
        }
    }
}

fn check_string_keys(
    map: &yaml_serde::Mapping,
    path: impl AsRef<str>,
    report: &mut ConfigValidationReport,
) {
    let path = path.as_ref();
    for key in map.keys() {
        if key.as_str().is_none() {
            report.error(path, "mapping keys must be strings");
        }
    }
}

fn mapping_get<'a>(map: &'a yaml_serde::Mapping, key: &str) -> Option<&'a yaml_serde::Value> {
    map.get(yaml_serde::Value::String(key.to_owned()))
}

fn string_entries(map: &yaml_serde::Mapping) -> impl Iterator<Item = (&str, &yaml_serde::Value)> {
    map.iter()
        .filter_map(|(key, value)| key.as_str().map(|key| (key, value)))
}

fn validate_agent_config(config: &AgentConfig, report: &mut ConfigValidationReport) {
    if config.default.trim().is_empty() {
        report.error("agent.default", "must not be empty");
    } else if !config.profiles.contains_key(&config.default) {
        report.error(
            "agent.default",
            format!("references unknown profile `{}`", config.default),
        );
    }
    if config.profiles.is_empty() {
        report.error("agent.profiles", "must define at least one profile");
    }
    for name in config.profiles.keys() {
        let path = format!("agent.profiles.{name}");
        if name.trim().is_empty() {
            report.error(&path, "profile name must not be empty");
        }
    }
    for (id, instance) in &config.instances {
        let path = format!("agent.instances.{id}");
        if id.trim().is_empty() {
            report.error(&path, "agent instance id must not be empty");
        }
        let profile = if instance.profile.trim().is_empty() {
            &config.default
        } else {
            &instance.profile
        };
        if !config.profiles.contains_key(profile) {
            report.error(
                format!("{path}.profile"),
                format!("references unknown profile `{profile}`"),
            );
        }
        if instance.session_policy.max_parallel_subagents == 0 {
            report.error(
                format!("{path}.session_policy.max_parallel_subagents"),
                "must be greater than 0",
            );
        }
        for (index, binding) in instance.route_bindings.iter().enumerate() {
            if binding.channel.trim().is_empty() {
                report.error(
                    format!("{path}.route_bindings[{index}].channel"),
                    "must not be empty",
                );
            }
        }
    }
}

fn validate_model_config(
    config: &ModelConfig,
    provider_settings: &RemoteProviderConfig,
    report: &mut ConfigValidationReport,
) {
    let provider = normalize(&config.provider);
    if !is_allowed_model_provider(&provider) {
        report.error(
            "model.default.provider",
            format!("unsupported model provider `{}`", config.provider),
        );
        return;
    }
    if config.runtime.trim() != "harness-agent-loop" {
        report.error(
            "model.default.runtime",
            "only `harness-agent-loop` is supported",
        );
    }
    validate_model_transport(&provider, &config.transport, report);
    validate_timeout("model.default.timeout_ms", config.timeout_ms, report);
    validate_optional_positive(
        "model.default.rate_limit_per_minute",
        config.rate_limit_per_minute,
        report,
    );
    validate_optional_positive(
        "model.default.daily_token_budget",
        config.daily_token_budget,
        report,
    );
    if provider == "mock" {
        if config.model.trim().is_empty() {
            report.warning(
                "model.default.model",
                "mock provider is selected with an empty model name",
            );
        }
        return;
    }
    validate_required("model.default.model", &config.model, report);
    if provider == "ollama" {
        validate_optional_url(
            "providers.model.base_url",
            &provider_settings.base_url,
            report,
        );
    } else {
        validate_required_url(
            "providers.model.base_url",
            &provider_settings.base_url,
            report,
        );
        validate_required(
            "providers.model.api_key",
            &provider_settings.api_key,
            report,
        );
    }
}

fn validate_model_transport(provider: &str, transport: &str, report: &mut ConfigValidationReport) {
    let transport = transport.trim();
    if transport.is_empty() {
        report.error("model.default.transport", "must not be empty");
        return;
    }
    let expected = match provider {
        "mock" => "mock",
        "openai-compatible" => "openai-compatible-chat-completions",
        "anthropic" => "anthropic-messages",
        "ollama" => "ollama-chat",
        _ => return,
    };
    if transport != expected {
        report.error(
            "model.default.transport",
            format!("provider `{provider}` requires transport `{expected}`"),
        );
    }
}

fn validate_policy_config(config: &PolicyConfig, report: &mut ConfigValidationReport) {
    validate_policy_value("policy.workspace_writes", &config.workspace_writes, report);
    validate_policy_value("policy.network", &config.network, report);
}

fn validate_memory_config(config: &MemoryConfig, report: &mut ConfigValidationReport) {
    validate_local_backend("memory.backend", &config.backend, report);
    let enabled = config
        .external_providers
        .iter()
        .filter(|provider| provider.enabled)
        .count();
    if enabled > 1 {
        report.error(
            "memory.external_providers",
            format!("only one external memory provider may be enabled, found {enabled}"),
        );
    }
    for (index, provider) in config.external_providers.iter().enumerate() {
        let path = format!("memory.external_providers[{index}]");
        if provider.id.trim().is_empty() {
            report.error(format!("{path}.id"), "must not be empty");
        }
        if provider.provider.trim() != "plugin" {
            report.error(format!("{path}.provider"), "only `plugin` is supported");
        }
        if provider.enabled {
            report.error(
                format!("{path}.enabled"),
                "external memory providers are descriptors only in the MVP and cannot be enabled",
            );
        }
        if let Some(endpoint) = &provider.endpoint {
            validate_url(format!("{path}.endpoint"), endpoint, report);
        }
    }
}

fn validate_rag_config(
    config: &RagConfig,
    provider_settings: &RemoteProviderConfig,
    report: &mut ConfigValidationReport,
) {
    validate_local_backend("rag.backend", &config.backend, report);
    let provider = normalize(&config.embedding_provider);
    if !is_allowed_embedding_provider(&provider) {
        report.error(
            "rag.embedding_provider",
            format!(
                "unsupported embedding provider `{}`",
                config.embedding_provider
            ),
        );
        return;
    }
    validate_timeout(
        "rag.embedding_timeout_ms",
        config.embedding_timeout_ms,
        report,
    );
    if is_remote_embedding_provider(&provider) {
        validate_required("rag.embedding_model", &config.embedding_model, report);
        validate_required_url(
            "providers.embedding.base_url",
            &provider_settings.base_url,
            report,
        );
        validate_required(
            "providers.embedding.api_key",
            &provider_settings.api_key,
            report,
        );
    }
}

fn validate_voice_config(
    path: &str,
    config: &VoiceProviderConfig,
    provider_settings: &RemoteProviderConfig,
    is_tts: bool,
    report: &mut ConfigValidationReport,
) {
    let provider = normalize(&config.provider);
    if !is_allowed_voice_provider(&provider) {
        report.error(
            format!("{path}.provider"),
            format!("unsupported voice provider `{}`", config.provider),
        );
        return;
    }
    validate_timeout(format!("{path}.timeout_ms"), config.timeout_ms, report);
    if provider == "mock" {
        return;
    }
    validate_required(format!("{path}.model"), &config.model, report);
    let provider_path = if is_tts {
        "providers.tts"
    } else {
        "providers.asr"
    };
    validate_required_url(
        format!("{provider_path}.base_url"),
        &provider_settings.base_url,
        report,
    );
    validate_required(
        format!("{provider_path}.api_key"),
        &provider_settings.api_key,
        report,
    );
    if is_tts {
        if let Some(voice) = &config.voice {
            if voice.trim().is_empty() {
                report.error(format!("{path}.voice"), "must not be empty when set");
            }
        }
    } else if config.voice.is_some() {
        report.warning(format!("{path}.voice"), "ASR ignores the voice field");
    }
}

fn validate_self_modify_config(config: &SelfModifyConfig, report: &mut ConfigValidationReport) {
    for (name, profile) in &config.check_profiles {
        let path = format!("self_modify.check_profiles.{name}");
        if name.trim().is_empty() {
            report.error(&path, "check profile name must not be empty");
        }
        if profile.commands.is_empty() {
            report.error(
                format!("{path}.commands"),
                "must contain at least one command",
            );
        }
        for (index, command) in profile.commands.iter().enumerate() {
            if command.trim().is_empty() {
                report.error(format!("{path}.commands[{index}]"), "must not be empty");
            }
        }
    }
}

fn validate_policy_value(
    path: impl Into<String>,
    value: &str,
    report: &mut ConfigValidationReport,
) {
    let path = path.into();
    if !matches!(normalize(value).as_str(), "allow" | "ask" | "deny") {
        report.error(path, "must be one of: allow, ask, deny");
    }
}

fn validate_local_backend(
    path: impl Into<String>,
    backend: &str,
    report: &mut ConfigValidationReport,
) {
    let path = path.into();
    if !matches!(normalize(backend).as_str(), "jsonl" | "sqlite") {
        report.error(path, "must be one of: jsonl, sqlite");
    }
}

fn validate_timeout(path: impl Into<String>, timeout_ms: u64, report: &mut ConfigValidationReport) {
    if timeout_ms == 0 {
        report.error(path.into(), "must be greater than 0");
    }
}

fn validate_optional_positive(
    path: impl Into<String>,
    value: Option<u32>,
    report: &mut ConfigValidationReport,
) {
    if value == Some(0) {
        report.error(path.into(), "must be greater than 0 when set");
    }
}

fn validate_required(path: impl Into<String>, value: &str, report: &mut ConfigValidationReport) {
    if value.trim().is_empty() {
        report.error(path.into(), "must not be empty");
    }
}

fn validate_required_url(
    path: impl Into<String>,
    value: &str,
    report: &mut ConfigValidationReport,
) {
    let path = path.into();
    if value.trim().is_empty() {
        report.error(path, "must not be empty");
        return;
    }
    validate_url(path, value, report);
}

fn validate_optional_url(
    path: impl Into<String>,
    value: &str,
    report: &mut ConfigValidationReport,
) {
    if value.trim().is_empty() {
        return;
    }
    validate_url(path, value, report);
}

fn validate_url(path: impl Into<String>, value: &str, report: &mut ConfigValidationReport) {
    let path = path.into();
    let trimmed = value.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        report.error(path, "must start with http:// or https://");
    }
}

fn is_allowed_model_provider(provider: &str) -> bool {
    matches!(
        provider,
        "mock" | "openai-compatible" | "anthropic" | "ollama"
    )
}

fn is_allowed_embedding_provider(provider: &str) -> bool {
    matches!(provider, "hash" | "sparse" | "mock" | "openai-compatible")
}

fn is_remote_embedding_provider(provider: &str) -> bool {
    provider == "openai-compatible"
}

fn is_allowed_voice_provider(provider: &str) -> bool {
    matches!(provider, "mock" | "openai-compatible")
}

fn normalize(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}
