// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use crate::support::TestHome;

#[test]
fn config_validate_reports_missing_config_yaml() {
    let env = TestHome::new();

    let output = env.run_failure(["config", "validate"]);

    assert!(output.contains("error: config:"));
    assert!(output.contains("config.yaml"));
    assert!(output.contains("configuration validation failed"));
}

#[test]
fn config_validate_accepts_offline_mock_config() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let output = env.run(["config", "validate"]);

    assert!(output.contains("config valid:"));
    assert!(output.contains("config.yaml"));
}

#[test]
fn config_validate_rejects_unknown_fields_without_leaking_secrets() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"providers:
  model:
    api_key: sk-local-secret
    base_url: https://api.example/v1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros
    old_alias_field: true

rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
  asr:
    provider: mock
"#,
    )
    .expect("write config");

    let output = env.run_failure(["config", "validate"]);

    assert!(output.contains("error: model.default.old_alias_field"));
    assert!(output.contains("unknown configuration field"));
    assert!(!output.contains("local-secret"));
}

#[test]
fn config_validate_rejects_missing_remote_model_settings() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions

rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
  asr:
    provider: mock
"#,
    )
    .expect("write config");

    let output = env.run_failure(["config", "validate"]);

    assert!(output.contains("error: model.default.model: must not be empty"));
    assert!(output.contains("error: providers.model.base_url: must not be empty"));
    assert!(output.contains("error: providers.model.api_key: must not be empty"));
}

#[test]
fn config_validate_rejects_enabled_external_memory_provider() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"model:
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

memory:
  external_providers:
    - id: remote-memory
      provider: plugin
      enabled: true
      endpoint: http://127.0.0.1:8787
      api_key: sk-memory-secret
"#,
    )
    .expect("write config");

    let output = env.run_failure(["config", "validate"]);

    assert!(output.contains("error: memory.external_providers[0].enabled"));
    assert!(output.contains("external memory providers are descriptors only"));
    assert!(!output.contains("memory-secret"));
}
