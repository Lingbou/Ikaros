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
