// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use crate::support::TestHome;

#[test]
fn provider_inspect_reports_registry_metadata_without_secret_values() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"providers:
  model:
    api_key: sk-secret-provider-key
    base_url: https://api.moonshot.cn/v1
  embedding:
    api_key: ""
    base_url: ""
  tts:
    api_key: ""
    base_url: ""
  asr:
    api_key: ""
    base_url: ""

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: kimi-k2.6

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

    let output = env.run(["provider", "inspect"]);

    assert!(output.contains("provider: openai-compatible"));
    assert!(output.contains("model: kimi-k2.6"));
    assert!(output.contains("profile: moonshot-kimi"));
    assert!(output.contains("context_window: 128000"));
    assert!(output.contains("streaming: true"));
    assert!(output.contains("tool_calls: true"));
    assert!(output.contains("network: true"));
    assert!(!output.contains("sk-secret-provider-key"));
}

#[test]
fn provider_health_reads_local_ledger_without_live_call_or_secret_leak() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"providers:
  model:
    api_key: sk-secret-provider-key
    base_url: https://api.moonshot.cn/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: kimi-k2.6

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

    let output = env.run(["provider", "health"]);

    assert!(output.contains("provider: openai-compatible"));
    assert!(output.contains("model: kimi-k2.6"));
    assert!(output.contains("health: Unknown"));
    assert!(output.contains("health_log:"));
    assert!(!output.contains("sk-secret-provider-key"));
}

#[test]
fn provider_matrix_reports_local_live_smoke_readiness_without_secret_values() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"providers:
  model:
    api_key: sk-secret-model-key
    base_url: https://api.moonshot.cn/v1
  embedding:
    api_key: sk-secret-embedding-key
    base_url: https://api.siliconflow.cn/v1
  tts:
    api_key: ""
    base_url: ""
  asr:
    api_key: ""
    base_url: ""

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: kimi-k2.6

rag:
  embedding_provider: openai-compatible
  embedding_model: BAAI/bge-m3

voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr
"#,
    )
    .expect("write config");

    let output = env.run(["provider", "matrix"]);

    assert!(output.contains("provider_matrix:"));
    assert!(output.contains("matrix_row: kind=model provider=openai-compatible model=kimi-k2.6"));
    assert!(output.contains("base_url_configured=true"));
    assert!(output.contains("api_key_configured=true"));
    assert!(output.contains("live_smoke=ready"));
    assert!(output.contains("live_probe=not-run"));
    assert!(output.contains("context_window=128000"));
    assert!(output.contains("streaming=true"));
    assert!(output.contains("tool_calls=true"));
    assert!(output.contains("reasoning=true"));
    assert!(output.contains("network=true"));
    assert!(output.contains("cost_input_per_million=unknown"));
    assert!(output.contains("cost_output_per_million=unknown"));
    assert!(
        output.contains("matrix_row: kind=embedding provider=openai-compatible model=BAAI/bge-m3")
    );
    assert!(output.contains("matrix_row: kind=tts provider=mock model=mock-tts"));
    assert!(output.contains("matrix_row: kind=asr provider=mock model=mock-asr"));
    assert!(output.contains("provider_profile=moonshot-kimi"));
    assert!(output.contains("provider_profile=mock"));
    assert!(!output.contains("sk-secret-model-key"));
    assert!(!output.contains("sk-secret-embedding-key"));
}

#[test]
fn provider_matrix_live_probes_mock_model_without_secret_values() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let output = env.run(["provider", "matrix", "--live"]);

    assert!(output.contains("provider_matrix: live=true"));
    assert!(output.contains("matrix_row: kind=model provider=mock"));
    assert!(output.contains("live_probe=ok"));
    assert!(output.contains("matrix_row: kind=embedding"));
    assert!(output.contains("live_probe=not-supported"));
    assert!(!output.contains("sk-"));
}
