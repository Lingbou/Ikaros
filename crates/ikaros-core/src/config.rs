// SPDX-License-Identifier: GPL-3.0-only

use crate::AgentConfig;
use serde::{Deserialize, Serialize};

mod execution;
mod init;
mod kinds;
mod mcp;
mod memory;
mod model;
mod policy;
mod presets;
mod providers;
mod rag;
mod resolved;
mod self_modify;
mod store;
mod validation;
mod voice;

pub use execution::{ExecutionConfig, ExecutionNetworkConfig, ExecutionSandboxConfig};
pub use kinds::{
    EmbeddingProviderKind, ModelProviderKind, ModelTransportKind, SandboxBackend, SandboxReadScope,
    StoreBackend, VoiceProviderKind,
};
pub use mcp::{McpConfig, McpServerConfig};
pub use memory::{ExternalMemoryProviderConfig, MemoryConfig, MemoryPolicyConfig};
pub use model::{
    ModelConfig, ModelCostConfig, ModelFallbackConfig, ModelParamsConfig, ModelReasoningConfig,
    ModelTable,
};
pub use policy::PolicyConfig;
pub use providers::{ExternalProvidersConfig, RemoteProviderConfig};
pub use rag::RagConfig;
pub use self_modify::{SelfModifyCheckProfileConfig, SelfModifyConfig};
pub use store::LocalStoreConfig;
pub use validation::{ConfigValidationIssue, ConfigValidationReport};
pub use voice::{VoiceConfig, VoiceProviderConfig};

pub const CURRENT_CONFIG_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct IkarosConfig {
    pub schema_version: u32,
    pub providers: ExternalProvidersConfig,
    pub agent: AgentConfig,
    pub model: ModelTable,
    pub policy: PolicyConfig,
    pub memory: MemoryConfig,
    pub rag: RagConfig,
    pub voice: VoiceConfig,
    pub mcp: McpConfig,
    pub execution: ExecutionConfig,
    pub self_modify: SelfModifyConfig,
}

impl Default for IkarosConfig {
    fn default() -> Self {
        Self {
            schema_version: CURRENT_CONFIG_SCHEMA_VERSION,
            providers: ExternalProvidersConfig::default(),
            agent: AgentConfig::default(),
            model: ModelTable::default(),
            policy: PolicyConfig::default(),
            memory: MemoryConfig::default(),
            rag: RagConfig::default(),
            voice: VoiceConfig::default(),
            mcp: McpConfig::default(),
            execution: ExecutionConfig::default(),
            self_modify: SelfModifyConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn top_level_provider_settings_are_schema_only() {
        let raw = r#"
schema_version: 1

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
"#;

        let config = validation::load_yaml_shape_checked(raw).expect("shape config");
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
    fn generated_default_config_has_valid_shape_but_requires_runtime_values() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        IkarosConfig::write_default_config(&path).expect("write");
        let raw = fs::read_to_string(&path).expect("read generated config");

        let config =
            validation::load_yaml_shape_checked(&raw).expect("generated config shape is valid");
        assert_eq!(raw.lines().count(), 8);
        assert!(raw.starts_with("schema_version: 1\n"));
        assert!(raw.contains("    preset: auto\n"));
        assert!(raw.contains("    api_key: \"\"\n"));
        assert!(raw.contains("    base_url: \"\"\n"));
        assert_eq!(config.schema_version, CURRENT_CONFIG_SCHEMA_VERSION);
        assert_eq!(config.model.default.preset.as_deref(), Some("auto"));
        assert_eq!(config.model.default.provider.as_str(), "openai-compatible");
        assert_eq!(config.model.default.api_key.as_deref(), Some(""));
        assert_eq!(config.model.default.base_url.as_deref(), Some(""));
        assert!(config.providers.model.api_key.is_empty());
        assert!(config.providers.model.base_url.is_empty());
        assert_eq!(config.rag.embedding_provider.as_str(), "hash");
        assert!(config.providers.embedding.base_url.is_empty());
        assert_eq!(config.voice.tts.provider.as_str(), "mock");
        assert!(config.providers.tts.base_url.is_empty());
        assert!(config.execution.network.enabled);
        assert!(config.execution.network.allow_provider_hosts);
        assert_eq!(config.execution.sandbox.backend.as_str(), "local");
        assert_eq!(config.execution.sandbox.read_scope.as_str(), "workspace");
        assert_eq!(config.model.default.daily_token_budget, None);
        assert!(config.mcp.servers.is_empty());

        let error = IkarosConfig::load(&path).expect_err("runtime load requires configured values");
        assert!(
            error
                .to_string()
                .contains("configuration validation failed")
        );
        assert!(error.to_string().contains("model.default.model"));
    }

    #[test]
    fn full_config_serializes_all_defaults() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        IkarosConfig::write_full_config(&path).expect("write");
        let raw = fs::read_to_string(&path).expect("read generated config");

        let config =
            validation::load_yaml_shape_checked(&raw).expect("generated config shape is valid");
        assert!(raw.lines().count() > 10);
        assert!(raw.contains("providers:"));
        assert!(raw.contains("agent:"));
        assert!(raw.contains("execution:"));
        assert_eq!(config.schema_version, CURRENT_CONFIG_SCHEMA_VERSION);
        assert_eq!(config.model.default.provider.as_str(), "openai-compatible");
        assert_eq!(config.rag.embedding_provider.as_str(), "hash");
        assert_eq!(config.voice.tts.provider.as_str(), "mock");
        assert_eq!(config.voice.asr.provider.as_str(), "mock");
    }

    #[test]
    fn inline_model_provider_settings_override_shared_pool() {
        let config = IkarosConfig {
            providers: ExternalProvidersConfig {
                model: RemoteProviderConfig {
                    api_key: "shared-key".into(),
                    base_url: "https://shared.example/v1".into(),
                },
                ..ExternalProvidersConfig::default()
            },
            model: ModelTable {
                default: ModelConfig {
                    api_key: Some("inline-key".into()),
                    base_url: Some("https://inline.example/v1".into()),
                    ..ModelConfig::default()
                },
            },
            ..IkarosConfig::default()
        };

        let provider = config.effective_model_provider();
        assert_eq!(provider.api_key, "inline-key");
        assert_eq!(provider.base_url, "https://inline.example/v1");
    }

    #[test]
    fn empty_inline_model_provider_settings_fall_back_to_shared_pool() {
        let config = IkarosConfig {
            providers: ExternalProvidersConfig {
                model: RemoteProviderConfig {
                    api_key: "shared-key".into(),
                    base_url: "https://shared.example/v1".into(),
                },
                ..ExternalProvidersConfig::default()
            },
            model: ModelTable {
                default: ModelConfig {
                    api_key: Some("  ".into()),
                    base_url: Some("".into()),
                    ..ModelConfig::default()
                },
            },
            ..IkarosConfig::default()
        };

        let provider = config.effective_model_provider();
        assert_eq!(provider.api_key, "shared-key");
        assert_eq!(provider.base_url, "https://shared.example/v1");
    }

    #[test]
    fn empty_rag_and_voice_sections_keep_local_defaults() {
        let report = IkarosConfig::validate_yaml(
            r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag: {}

voice:
  tts: {}
  asr: {}
"#,
        )
        .expect("validate yaml");

        assert!(report.is_valid(), "{report:#?}");

        let config = IkarosConfig::load_yaml_shape_checked(
            r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag: {}

voice:
  tts: {}
  asr: {}
"#,
        )
        .expect("shape config");
        assert_eq!(config.rag.embedding_provider.as_str(), "hash");
        assert_eq!(config.voice.tts.provider.as_str(), "mock");
        assert_eq!(config.voice.asr.provider.as_str(), "mock");
    }

    #[test]
    fn config_validation_accepts_configured_mcp_stdio_servers() {
        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr

mcp:
  servers:
    - id: local-tools
      enabled: false
      transport: stdio
      command: /usr/bin/example-mcp
      args: ["--stdio"]
      include_tools: ["search"]
      exclude_tools: []
      timeout_ms: 5000
      max_output_bytes: 65536
"#,
        )
        .expect("validate");

        assert!(report.is_valid(), "{report:#?}");
    }

    #[test]
    fn config_validation_rejects_unsafe_mcp_stdio_server_shape() {
        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr

mcp:
  servers:
    - id: tools
      enabled: true
      transport: http
      command: "mcp-server; rm -rf /"
      include_tools: ["search", "search"]
      exclude_tools: ["search"]
      timeout_ms: 0
      max_output_bytes: 0
"#,
        )
        .expect("validate");

        for path in [
            "mcp.servers[0].transport",
            "mcp.servers[0].command",
            "mcp.servers[0].include_tools[1]",
            "mcp.servers[0].exclude_tools",
            "mcp.servers[0].timeout_ms",
            "mcp.servers[0].max_output_bytes",
        ] {
            assert!(
                report.errors.iter().any(|issue| issue.path == path),
                "{path} missing from {report:#?}"
            );
        }
    }

    #[test]
    fn config_validation_rejects_unknown_schema_version() {
        let raw = r#"
schema_version: 999
providers:
  model:
    api_key: key
    base_url: https://model.example/v1
model:
  default:
    provider: openai-compatible
    transport: openai-compatible-chat-completions
    model: test-model
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: ""
voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr
"#;

        let report = IkarosConfig::validate_yaml(raw).expect("validate");
        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "schema_version")
        );
    }

    #[test]
    fn config_validation_requires_explicit_schema_version() {
        let raw = r#"
providers:
  model:
    api_key: key
    base_url: https://model.example/v1
model:
  default:
    provider: openai-compatible
    transport: openai-compatible-chat-completions
    model: test-model
rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: ""
voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr
"#;

        let report = IkarosConfig::validate_yaml(raw).expect("validate");
        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "config.schema_version"),
            "{report:#?}"
        );
    }

    #[test]
    fn generated_default_config_does_not_include_legacy_chat_history_backend() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        IkarosConfig::write_default_config(&path).expect("write");
        let raw = fs::read_to_string(&path).expect("read generated config");

        assert!(
            !raw.contains("\nchat_history:"),
            "chat timeline is session-store-only; default config must not expose legacy history backend knobs"
        );
    }

    #[test]
    fn config_load_rejects_unknown_fields() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        fs::write(
            &path,
            r#"
schema_version: 1

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
schema_version: 1

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
    fn model_cost_config_accepts_cache_pricing_and_rejects_negative_values() {
        let raw = r#"
schema_version: 1

providers:
  model:
    api_key: model-secret
    base_url: https://api.example/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: priced-chat
    cost:
      currency: CNY
      input_per_million: 4.0
      output_per_million: 16.0
      cache_read_per_million: 0.4
      cache_write_per_million: 4.0

rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr
"#;
        let config = validation::load_yaml_shape_checked(raw).expect("shape");

        assert_eq!(config.model.default.cost.currency, "CNY");
        assert_eq!(config.model.default.cost.input_per_million, Some(4.0));
        assert_eq!(config.model.default.cost.output_per_million, Some(16.0));
        assert_eq!(config.model.default.cost.cache_read_per_million, Some(0.4));
        assert_eq!(config.model.default.cost.cache_write_per_million, Some(4.0));

        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

providers:
  model:
    api_key: model-secret
    base_url: https://api.example/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: priced-chat
    cost:
      input_per_million: -1.0

rag:
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

        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "model.default.cost.input_per_million"),
            "{report:#?}"
        );
    }

    #[test]
    fn config_validation_does_not_inherit_tts_voice_into_asr() {
        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

providers:
  model:
    api_key: model-secret
    base_url: https://api.example/v1
  tts:
    api_key: tts-secret
    base_url: https://tts.example/v1
  asr:
    api_key: asr-secret
    base_url: https://asr.example/v1

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
    provider: openai-compatible
    model: speech-model
  asr:
    provider: openai-compatible
    model: transcription-model
"#,
        )
        .expect("validate");

        assert!(report.is_valid(), "{report:#?}");
        assert!(
            !report
                .warnings
                .iter()
                .any(|issue| issue.path == "voice.asr.voice"),
            "ASR should not inherit the TTS voice default: {report:#?}"
        );
    }

    #[test]
    fn config_validation_rejects_invalid_model_profile_options() {
        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

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
schema_version: 1

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
schema_version: 1

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
schema_version: 1

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
    fn config_validation_rejects_deferred_toolsets_without_core_bridge() {
        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
  asr:
    provider: mock

agent:
  default: build
  profiles:
    build:
      toolsets: [rag]
"#,
        )
        .expect("validate");

        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "agent.profiles.build.toolsets"),
            "{report:#?}"
        );
    }

    #[test]
    fn config_validation_rejects_missing_remote_settings() {
        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

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
    fn config_validation_checks_agent_instance_provider_override_without_model_override() {
        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

providers:
  model:
    api_key: global-key
    base_url: https://global.example/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: global-model

agent:
  default: build
  instances:
    coder:
      profile: build
      providers:
        model:
          api_key: ""
          base_url: ""

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

        assert!(report.errors.iter().any(|issue| {
            issue.path == "agent.instances.coder.providers.model.base_url"
                && issue.message.contains("must not be empty")
        }));
        assert!(report.errors.iter().any(|issue| {
            issue.path == "agent.instances.coder.providers.model.api_key"
                && issue.message.contains("must not be empty")
        }));
    }

    #[test]
    fn config_validation_rejects_enabled_external_memory_provider() {
        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

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

    #[test]
    fn config_load_rejects_semantically_invalid_yaml() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        fs::write(
            &path,
            r#"
schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr

memory:
  external_providers:
    - id: remote
      provider: plugin
      enabled: true
"#,
        )
        .expect("write config");

        let error = IkarosConfig::load(&path).expect_err("enabled external memory must fail load");
        assert!(
            error
                .to_string()
                .contains("configuration validation failed")
        );
        assert!(
            error
                .to_string()
                .contains("memory.external_providers[0].enabled")
        );
    }

    #[test]
    fn config_validation_rejects_invalid_memory_policy() {
        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

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
  policy:
    promote_threshold: 1.2
    demote_threshold: 0.8
    forget_threshold: 0.9
    max_records_per_scope: 0
"#,
        )
        .expect("validate");

        assert!(report.errors.iter().any(|issue| {
            issue.path == "memory.policy.promote_threshold"
                && issue.message.contains("between 0.0 and 1.0")
        }));
        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "memory.policy.forget_threshold")
        );
        assert!(
            report
                .errors
                .iter()
                .any(|issue| issue.path == "memory.policy.max_records_per_scope")
        );
    }

    #[test]
    fn config_validation_rejects_invalid_execution_boundary_config() {
        let report = IkarosConfig::validate_yaml(
            r#"
schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
  asr:
    provider: mock

execution:
  network:
    allowed_hosts:
      - https://api.example/v1
    timeout_ms: 0
  sandbox:
    backend: docker
    image: ""
    read_scope: host
"#,
        )
        .expect("validate");

        for path in [
            "execution.network.allowed_hosts[0]",
            "execution.network.timeout_ms",
            "execution.sandbox.image",
            "execution.sandbox.read_scope",
        ] {
            assert!(
                report.errors.iter().any(|issue| issue.path == path),
                "{path} missing from {report:#?}"
            );
        }
    }

    #[test]
    fn preset_expands_provider_and_transport_at_load() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"schema_version: 1

providers:
  model:
    api_key: test-key
    base_url: https://api.moonshot.cn/v1

model:
  default:
    preset: kimi
    model: kimi-k2.6

{}
"#,
                mock_rag_and_voice_yaml()
            ),
        )
        .expect("write config");

        let config = IkarosConfig::load(&path).expect("load");
        assert_eq!(config.model.default.provider.as_str(), "openai-compatible");
        assert_eq!(
            config.model.default.transport.as_str(),
            "openai-compatible-chat-completions"
        );
        assert_eq!(config.model.default.compat_profile, "moonshot-kimi");
    }

    #[test]
    fn preset_does_not_override_explicit_provider() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"schema_version: 1

providers:
  model:
    api_key: test-key
    base_url: https://api.anthropic.com

model:
  default:
    preset: kimi
    provider: anthropic
    transport: anthropic-messages
    model: claude-sonnet-4-5

{}
"#,
                mock_rag_and_voice_yaml()
            ),
        )
        .expect("write config");

        let config = IkarosConfig::load(&path).expect("load");
        assert_eq!(config.model.default.provider.as_str(), "anthropic");
        assert_eq!(config.model.default.compat_profile, "auto");
    }

    #[test]
    fn preset_auto_keeps_auto_detection() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        fs::write(
            &path,
            format!(
                r#"schema_version: 1

providers:
  model:
    api_key: test-key
    base_url: https://api.example.com/v1

model:
  default:
    preset: auto
    provider: openai-compatible
    model: test-model

{}
"#,
                mock_rag_and_voice_yaml()
            ),
        )
        .expect("write config");

        let config = IkarosConfig::load(&path).expect("load");
        assert_eq!(config.model.default.compat_profile, "auto");
    }

    #[test]
    fn minimal_config_validates_after_model_fields_are_filled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        IkarosConfig::write_default_config(&path).expect("write");
        let mut config = IkarosConfig::load_shape_checked(&path).expect("shape");
        config.model.default.model = "kimi-k2.6".into();
        config.model.default.api_key = Some("test-key".into());
        config.model.default.base_url = Some("https://api.moonshot.cn/v1".into());
        config.expand_presets();

        let report = config.validate();

        assert!(report.is_valid(), "{report:#?}");
        assert_eq!(config.rag.embedding_provider.as_str(), "hash");
        assert_eq!(config.voice.tts.provider.as_str(), "mock");
        assert_eq!(config.voice.asr.provider.as_str(), "mock");
    }

    #[test]
    fn native_provider_presets_validate_their_native_profiles() {
        for (preset, model, api_key, base_url) in [
            (
                "anthropic",
                "claude-sonnet-4-5",
                "test-key",
                "https://api.anthropic.com",
            ),
            ("ollama", "llama3.2", "", ""),
        ] {
            let raw = format!(
                r#"schema_version: 1

model:
  default:
    preset: {preset}
    model: {model}
    api_key: "{api_key}"
    base_url: "{base_url}"
"#,
            );

            let report = IkarosConfig::validate_yaml(&raw).expect("validate yaml");

            assert!(report.is_valid(), "{preset}: {report:#?}");
        }
    }

    #[test]
    fn unknown_preset_is_reported_during_validation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("config.yaml");
        fs::write(
            &path,
            r#"schema_version: 1

providers:
  model:
    api_key: test-key
    base_url: https://api.example.com/v1

model:
  default:
    preset: not-a-provider
    model: test-model
"#,
        )
        .expect("write config");

        let error = IkarosConfig::load(&path).expect_err("unknown preset should fail");
        let message = error.to_string();
        assert!(message.contains("model.default.preset"), "{message}");
        assert!(message.contains("unknown model preset"), "{message}");
    }

    fn mock_rag_and_voice_yaml() -> &'static str {
        r#"rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr"#
    }
}
