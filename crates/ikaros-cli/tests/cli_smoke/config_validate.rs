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
fn init_writes_minimal_config_by_default() {
    let env = TestHome::new();
    env.init();

    let raw = fs::read_to_string(env.home.join("config.yaml")).expect("read config");
    assert!(raw.lines().count() <= 10, "{raw}");
    assert!(raw.contains("preset: auto"));
    assert!(raw.contains("api_key: \"\""));
    assert!(!raw.contains("providers:"));
    assert!(!raw.contains("#"));
}

#[test]
fn init_full_writes_complete_default_config() {
    let env = TestHome::new();
    env.run(["init", "--full"]);

    let raw = fs::read_to_string(env.home.join("config.yaml")).expect("read config");
    assert!(raw.lines().count() > 10, "{raw}");
    assert!(raw.contains("providers:"));
    assert!(raw.contains("agent:"));
    assert!(raw.contains("execution:"));
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
        r#"schema_version: 1

providers:
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
fn config_validate_json_reports_machine_readable_errors_without_leaking_secrets() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

providers:
  model:
    api_key: sk-local-secret
    base_url: https://api.example/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: ""

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

    let output = env.run_failure(["config", "validate", "--json"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("json report");

    assert_eq!(report["valid"], serde_json::json!(false));
    assert_eq!(
        report["path"],
        serde_json::json!(env.home.join("config.yaml").display().to_string())
    );
    assert!(
        report["errors"]
            .as_array()
            .expect("errors")
            .iter()
            .any(|issue| issue["path"] == "model.default.model"
                && issue["message"] == "must not be empty"),
        "{report:#}"
    );
    assert!(!output.contains("local-secret"));
    assert!(!output.contains("sk-"));
}

#[test]
fn config_validate_rejects_missing_remote_model_settings() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

model:
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
        r#"schema_version: 1

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

#[test]
fn config_show_prints_redacted_runtime_summary() {
    let env = TestHome::new();
    env.run([
        "setup",
        "--api-key",
        "sk-local-secret",
        "--base-url",
        "https://api.example/v1",
        "--model",
        "configured-model",
        "--daily-token-budget",
        "12345",
    ]);

    let output = env.run(["config", "show"]);

    assert!(output.contains("config:"));
    assert!(output.contains("schema_version: 1"));
    assert!(output.contains("model_provider: openai-compatible"));
    assert!(output.contains("model_transport: openai-compatible-chat-completions"));
    assert!(output.contains("model_model: configured-model"));
    assert!(output.contains("model_api_key_configured: true"));
    assert!(output.contains("model_base_url_configured: true"));
    assert!(output.contains("model_daily_token_budget: 12345"));
    assert!(output.contains("memory_backend: jsonl"));
    assert!(output.contains("rag_backend: jsonl"));
    assert!(output.contains("rag_embedding_provider: hash"));
    assert!(output.contains("voice_tts_provider: mock"));
    assert!(output.contains("voice_asr_provider: mock"));
    assert!(output.contains("execution_network_enabled: true"));
    assert!(output.contains("execution_sandbox_backend: local"));
    assert!(!output.contains("sk-local-secret"));
    assert!(!output.contains("https://api.example/v1"));

    let json = env.run(["config", "show", "--json"]);
    let report: serde_json::Value = serde_json::from_str(&json).expect("json report");
    assert_eq!(report["schema_version"], serde_json::json!(1));
    assert_eq!(report["model"]["provider"], "openai-compatible");
    assert_eq!(report["model"]["model"], "configured-model");
    assert_eq!(report["model"]["api_key_configured"], true);
    assert_eq!(report["model"]["base_url_configured"], true);
    assert_eq!(report["model"]["daily_token_budget"], 12345);
    assert_eq!(report["rag"]["embedding_provider"], "hash");
    assert_eq!(report["voice"]["tts"]["provider"], "mock");
    assert!(!json.contains("sk-local-secret"));
    assert!(!json.contains("https://api.example/v1"));
}
