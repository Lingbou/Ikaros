// SPDX-License-Identifier: GPL-3.0-only

use crate::{AgentConfig, IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fs, path::Path};

mod validation;

pub use validation::{ConfigValidationIssue, ConfigValidationReport};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
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
        validation::load_yaml_shape_checked(&raw)
    }

    pub fn write_default_config(path: &Path) -> Result<()> {
        let raw = r#"# Ikaros local runtime configuration.
# This file belongs under IKAROS_HOME and may contain plaintext local credentials.
# Start by filling the three empty values below: api_key, base_url, and model.

providers:
  model:
    # API key for the default chat/completion provider.
    api_key: ""
    # Base URL for the default chat/completion provider.
    # Example: https://api.example.com/v1.
    base_url: ""

model:
  default:
    # Model identifier sent to the provider.
    model: ""
    # Provider family: openai-compatible, anthropic, ollama, or mock for tests only.
    provider: openai-compatible
    # Agent runtime implementation that owns the turn loop.
    runtime: harness-agent-loop
    # Wire protocol used by the provider adapter.
    transport: openai-compatible-chat-completions
    # Provider compatibility profile: auto, generic, moonshot-kimi, deepseek,
    # gemini-openai, openrouter, qwen, or local-openai-compatible.
    # auto prefers base_url detection, then model-name hints, then generic.
    compat_profile: auto
    params:
      # Maximum output tokens. null means the adapter/profile may choose a provider default.
      max_tokens: null
      # Sampling temperature. null means do not send temperature unless a profile requires it.
      temperature: null
      # Optional nucleus sampling value. null means do not send top_p.
      top_p: null
      # Number of completions to request when supported. null means provider default.
      n: null
      # OpenAI-compatible presence penalty. null means provider default.
      presence_penalty: null
      # OpenAI-compatible frequency penalty. null means provider default.
      frequency_penalty: null
      # Optional deterministic seed when the provider supports it.
      seed: null
      # Stop sequences sent to providers that support OpenAI-compatible stop.
      stop: []
    reasoning:
      # true enables provider-native thinking/reasoning when the profile supports it.
      # false asks the profile to disable it. null leaves the profile default.
      enabled: null
      # Reasoning effort: none, minimal, low, medium, high, xhigh, or max.
      effort: null
    # Extra JSON object merged into the final provider request body by the adapter.
    # Put provider-specific non-secret knobs here; secret-like values are redacted in logs.
    extra_body: {}
    # Request timeout in milliseconds.
    timeout_ms: 30000
    # Provider retry count after the first failed attempt.
    max_retries: 0
    # Optional per-minute request budget for model calls.
    rate_limit_per_minute: 60
    # Optional daily token budget recorded by the usage ledger.
    daily_token_budget: 100000

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
  embedding_provider: hash
  # Embedding model name sent to the provider.
  embedding_model: ""
  # Embedding request timeout in milliseconds.
  embedding_timeout_ms: 30000
  # Provider retry count for embedding calls.
  embedding_max_retries: 0

voice:
  tts:
    # Text-to-speech provider: mock or openai-compatible.
    provider: mock
    # TTS model name sent to the provider.
    model: mock-tts
    # TTS request timeout in milliseconds.
    timeout_ms: 30000
    # Provider retry count for TTS calls.
    max_retries: 0
    # Default TTS voice name.
    voice: default
  asr:
    # Speech-to-text provider: mock or openai-compatible.
    provider: mock
    # ASR model name sent to the provider.
    model: mock-asr
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
        assert_eq!(config.rag.embedding_provider, "hash");
        assert!(config.providers.embedding.base_url.is_empty());
        assert_eq!(config.voice.tts.provider, "mock");
        assert!(config.providers.tts.base_url.is_empty());
    }

    #[test]
    fn generated_default_config_keeps_initial_model_settings_near_top() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        IkarosConfig::write_default_config(&path).expect("write");

        let raw = fs::read_to_string(&path).expect("read generated config");
        let lines = raw.lines().collect::<Vec<_>>();
        let api_key_line = line_number(&lines, "    api_key: \"\"");
        let base_url_line = line_number(&lines, "    base_url: \"\"");
        let model_line = line_number(&lines, "    model: \"\"");

        assert!(api_key_line <= 8, "api_key should be near top");
        assert_eq!(base_url_line, api_key_line + 3);
        assert_eq!(model_line, base_url_line + 5);
    }

    #[test]
    fn config_load_rejects_unknown_fields() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        fs::write(
            &path,
            r#"
model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros
    old_alias_field: true
"#,
        )
        .expect("write");

        let error = IkarosConfig::load(&path).expect_err("unknown field rejected");

        assert!(
            error
                .to_string()
                .contains("configuration shape validation failed")
        );
        assert!(error.to_string().contains("model.default.old_alias_field"));
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
    compat_profile: moonshot-kimi
    params:
      max_tokens: 4096
      temperature: 0.2
      top_p: 0.9
      n: 1
      presence_penalty: 0.0
      frequency_penalty: 0.0
      seed: 42
      stop:
        - END
    reasoning:
      enabled: true
      effort: high
    extra_body:
      service_tier: standard
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
    fn config_validation_rejects_invalid_model_profile_options() {
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
    compat_profile: old-kimi-alias
    params:
      temperature: 4.0
      top_p: 2.0
      n: 0
      stop:
        - ""
    reasoning:
      effort: huge
    extra_body: {}

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

        for path in [
            "model.default.compat_profile",
            "model.default.params.temperature",
            "model.default.params.top_p",
            "model.default.params.n",
            "model.default.params.stop[0]",
            "model.default.reasoning.effort",
        ] {
            assert!(
                report.errors.iter().any(|issue| issue.path == path),
                "{path} missing from {report:#?}"
            );
        }
    }

    #[test]
    fn config_validation_rejects_non_object_model_extra_body_shape() {
        let report = IkarosConfig::validate_yaml(
            r#"
model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros
    extra_body: not-an-object
"#,
        )
        .expect("validate");

        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "model.default.extra_body"),
            "{report:#?}"
        );
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
    fn config_validation_rejects_unknown_fields() {
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
    }

    #[test]
    fn config_validation_rejects_missing_remote_settings() {
        let report = IkarosConfig::validate_yaml(
            r#"
model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: example-chat
"#,
        )
        .expect("validate");

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

    fn line_number(lines: &[&str], needle: &str) -> usize {
        lines
            .iter()
            .position(|line| *line == needle)
            .map(|index| index + 1)
            .unwrap_or_else(|| panic!("missing generated config line: {needle}"))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelTable {
    pub default: ModelConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelConfig {
    pub provider: String,
    pub runtime: String,
    pub transport: String,
    pub model: String,
    pub compat_profile: String,
    pub params: ModelParamsConfig,
    pub reasoning: ModelReasoningConfig,
    pub extra_body: serde_json::Map<String, serde_json::Value>,
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
            compat_profile: "auto".into(),
            params: ModelParamsConfig::default(),
            reasoning: ModelReasoningConfig::default(),
            extra_body: serde_json::Map::new(),
            timeout_ms: 30_000,
            max_retries: 0,
            rate_limit_per_minute: None,
            daily_token_budget: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelParamsConfig {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub n: Option<u32>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub seed: Option<u64>,
    pub stop: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct ModelReasoningConfig {
    pub enabled: Option<bool>,
    pub effort: Option<String>,
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
            embedding_provider: "hash".into(),
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
            tts: VoiceProviderConfig::mock_tts(),
            asr: VoiceProviderConfig::mock_asr(),
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
