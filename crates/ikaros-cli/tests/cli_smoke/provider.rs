// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use crate::support::TestHome;
use ikaros_models::{ModelUsageLedger, ModelUsageRecord};

#[test]
fn provider_inspect_reports_registry_metadata_without_secret_values() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

providers:
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
    cost:
      currency: CNY
      input_per_million: 4.0
      output_per_million: 16.0
      cache_read_per_million: 0.4
      cache_write_per_million: 4.0
    fallbacks:
      - provider: mock
        runtime: harness-agent-loop
        transport: mock
        model: fallback-mock

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
    assert!(output.contains("configured_profile: auto"));
    assert!(output.contains("profile: moonshot-kimi"));
    assert!(output.contains("profile_source: auto-detected"));
    assert!(output.contains("temperature_policy: omit"));
    assert!(output.contains("reasoning_policy: moonshot-kimi"));
    assert!(output.contains("message_policy: plain"));
    assert!(output.contains("tool_schema_policy: moonshot-subset"));
    assert!(output.contains("request_body_policy: none"));
    assert!(output.contains("retry_without_parameters: none"));
    assert!(output.contains("fallback_count: 1"));
    assert!(output.contains(
        "fallback_row: index=0 provider=mock model=fallback-mock configured_profile=auto profile=mock live_smoke=offline"
    ));
    assert!(output.contains("context_window: 128000"));
    assert!(output.contains("streaming: true"));
    assert!(output.contains("tool_calls: true"));
    assert!(output.contains("network: true"));
    assert!(output.contains("cost_input_per_million: 4 CNY"));
    assert!(output.contains("cost_output_per_million: 16 CNY"));
    assert!(output.contains("cost_cache_read_per_million: 0.4 CNY"));
    assert!(output.contains("cost_cache_write_per_million: 4 CNY"));
    assert!(!output.contains("sk-secret-provider-key"));
}

#[test]
fn provider_profiles_lists_openai_compatible_profile_catalog() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let output = env.run(["provider", "profiles"]);

    assert!(output.contains("provider_profiles: openai-compatible"));
    assert!(output.contains("profile_row: provider=openai-compatible profile=generic"));
    assert!(output.contains("profile_row: provider=openai-compatible profile=moonshot-kimi"));
    assert!(output.contains("profile_row: provider=openai-compatible profile=qwen"));
    assert!(output.contains("auto_base_url_markers=api.moonshot.ai,api.moonshot.cn,api.kimi.com"));
    assert!(output.contains("auto_model_tail_prefixes=kimi-,kimi_"));
    assert!(output.contains("temperature_policy=omit"));
    assert!(output.contains("reasoning_policy=moonshot-kimi"));
    assert!(output.contains("tool_schema_policy=moonshot-subset"));
    assert!(output.contains("context_window=128000"));
    assert!(output.contains("network=true"));
}

#[test]
fn provider_inspect_honors_agent_instance_model_override() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

providers:
  model:
    api_key: ""
    base_url: ""

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: global-mock

agent:
  default: build
  instances:
    coder:
      profile: build
      model:
        provider: mock
        runtime: harness-agent-loop
        transport: mock
        model: instance-mock

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

    let global = env.run(["provider", "inspect"]);
    assert!(global.contains("provider: mock"));
    assert!(global.contains("model: global-mock"));

    let instance = env.run(["--agent", "coder", "provider", "inspect"]);
    assert!(instance.contains("provider: mock"));
    assert!(instance.contains("model: instance-mock"));
}

#[test]
fn provider_inspect_and_matrix_honor_explicit_openai_compat_profile() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

providers:
  model:
    api_key: sk-secret-provider-key
    base_url: https://example.invalid/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: generic-chat
    compat_profile: local-openai-compatible

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

    let inspect = env.run(["provider", "inspect"]);
    assert!(inspect.contains("configured_profile: local-openai-compatible"));
    assert!(inspect.contains("profile: local-openai-compatible"));
    assert!(inspect.contains("profile_source: explicit"));
    assert!(inspect.contains("context_window: 131072"));
    assert!(inspect.contains("default_output_tokens: 65536"));
    assert!(inspect.contains("network: false"));
    assert!(!inspect.contains("sk-secret-provider-key"));

    let matrix = env.run(["provider", "matrix"]);
    assert!(matrix.contains("configured_profile=local-openai-compatible"));
    assert!(matrix.contains("provider_profile=local-openai-compatible"));
    assert!(matrix.contains("profile_source=explicit"));
    assert!(matrix.contains("context_window=131072"));
    assert!(matrix.contains("default_output_tokens=65536"));
    assert!(matrix.contains("network=false"));
    assert!(!matrix.contains("sk-secret-provider-key"));
}

#[test]
fn provider_health_reads_local_ledger_without_live_call_or_secret_leak() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

providers:
  model:
    api_key: sk-secret-provider-key
    base_url: https://api.moonshot.cn/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: kimi-k2.6
    cost:
      currency: CNY
      input_per_million: 4.0
      output_per_million: 16.0
      cache_read_per_million: 0.4
      cache_write_per_million: 4.0
    fallbacks:
      - provider: mock
        runtime: harness-agent-loop
        transport: mock
        model: fallback-mock

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
        r#"schema_version: 1

providers:
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
    cost:
      currency: CNY
      input_per_million: 4.0
      output_per_million: 16.0
      cache_read_per_million: 0.4
      cache_write_per_million: 4.0
    fallbacks:
      - provider: mock
        runtime: harness-agent-loop
        transport: mock
        model: fallback-mock

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
    let today = time::OffsetDateTime::now_utc().date().to_string();
    ModelUsageLedger::new(env.home.join("audit"))
        .append(ModelUsageRecord {
            id: "provider-matrix-usage".into(),
            at: format!("{today}T04:00:00Z"),
            provider: "openai-compatible".into(),
            model: "kimi-k2.6".into(),
            prompt_tokens: Some(100),
            completion_tokens: Some(50),
            total_tokens: 150,
            cache_read_tokens: Some(20),
            cache_write_tokens: Some(10),
            estimated: false,
        })
        .expect("append usage");

    let output = env.run(["provider", "matrix"]);

    assert!(output.contains("provider_matrix:"));
    assert!(output.contains("matrix_row: kind=model provider=openai-compatible model=kimi-k2.6"));
    assert!(output.contains("base_url_configured=true"));
    assert!(output.contains("api_key_configured=true"));
    assert!(output.contains("live_smoke=ready"));
    assert!(output.contains("live_probe=not-run"));
    assert!(output.contains("fallback_role=primary"));
    assert!(output.contains("fallback_count=1"));
    assert!(output.contains("fallback_models=fallback-mock"));
    assert!(output.contains("debug_hint=ready"));
    assert!(output.contains("context_window=128000"));
    assert!(output.contains("temperature_policy=omit"));
    assert!(output.contains("reasoning_policy=moonshot-kimi"));
    assert!(output.contains("tool_schema_policy=moonshot-subset"));
    assert!(output.contains("retry_without_parameters=none"));
    assert!(output.contains("streaming=true"));
    assert!(output.contains("tool_calls=true"));
    assert!(output.contains("reasoning=true"));
    assert!(output.contains("network=true"));
    assert!(output.contains("cost_input_per_million=4"));
    assert!(output.contains("cost_output_per_million=16"));
    assert!(output.contains("cost_cache_read_per_million=0.4"));
    assert!(output.contains("cost_cache_write_per_million=4"));
    assert!(output.contains("cost_currency=CNY"));
    assert!(output.contains("usage_requests_today=1"));
    assert!(output.contains("usage_prompt_tokens_today=100"));
    assert!(output.contains("usage_completion_tokens_today=50"));
    assert!(output.contains("usage_total_tokens_today=150"));
    assert!(output.contains("cache_read_tokens_today=20"));
    assert!(output.contains("cache_write_tokens_today=10"));
    assert!(output.contains("estimated_cost_today=0.001128"));
    assert!(output.contains("cache_accounting=priced"));
    assert!(
        output.contains("matrix_row: kind=embedding provider=openai-compatible model=BAAI/bge-m3")
    );
    assert!(output.contains("matrix_row: kind=tts provider=mock model=mock-tts"));
    assert!(output.contains("matrix_row: kind=asr provider=mock model=mock-asr"));
    assert!(output.contains("fallback_role=not-applicable"));
    assert!(output.contains("fallback_count=0"));
    assert!(output.contains("fallback_models=none"));
    assert!(output.contains("debug_hint=offline-provider"));
    assert!(output.contains("provider_profile=moonshot-kimi"));
    assert!(output.contains("provider_profile=mock"));
    assert!(!output.contains("sk-secret-model-key"));
    assert!(!output.contains("sk-secret-embedding-key"));
}

#[test]
fn debug_provider_reports_structured_provider_diagnostics_without_secret_values() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

providers:
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
    fallbacks:
      - provider: mock
        runtime: harness-agent-loop
        transport: mock
        model: fallback-mock

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

    let output = env.run(["debug", "provider"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("provider debug json");

    assert_eq!(report["format"], "ikaros-provider-debug-v1");
    assert_eq!(report["model"]["provider"], "openai-compatible");
    assert_eq!(report["model"]["model"], "kimi-k2.6");
    assert_eq!(report["model"]["configured_profile"], "auto");
    assert_eq!(report["model"]["provider_profile"], "moonshot-kimi");
    assert_eq!(report["model"]["profile_source"], "auto-detected");
    assert_eq!(report["model"]["live_smoke"], "ready");
    assert_eq!(report["model"]["policy"]["temperature"], "omit");
    assert_eq!(report["model"]["policy"]["reasoning"], "moonshot-kimi");
    assert_eq!(report["model"]["context"]["context_window"], 128000);
    assert_eq!(report["model"]["capabilities"]["tool_calls"], true);
    assert_eq!(report["model"]["health"]["status"], "Unknown");
    assert_eq!(
        report["fallback_chain"]
            .as_array()
            .expect("fallbacks")
            .len(),
        1
    );
    assert_eq!(report["fallback_chain"][0]["provider"], "mock");
    assert_eq!(report["fallback_chain"][0]["model"], "fallback-mock");
    assert_eq!(report["fallback_chain"][0]["live_smoke"], "offline");
    assert!(
        report["matrix"]
            .as_array()
            .expect("matrix")
            .iter()
            .any(|row| row["kind"] == "embedding"
                && row["provider"] == "openai-compatible"
                && row["model"] == "BAAI/bge-m3"
                && row["live_smoke"] == "ready")
    );
    assert!(
        report["matrix"]
            .as_array()
            .expect("matrix")
            .iter()
            .any(|row| row["kind"] == "tts"
                && row["provider"] == "mock"
                && row["live_smoke"] == "offline")
    );
    assert!(!output.contains("sk-secret-model-key"));
    assert!(!output.contains("sk-secret-embedding-key"));
    assert!(!output.contains("api.moonshot.cn"));
    assert!(!output.contains("api.siliconflow.cn"));
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
    assert!(output.contains("matrix_row: kind=embedding provider=hash"));
    assert!(output.contains("matrix_row: kind=tts provider=mock"));
    assert!(output.contains("matrix_row: kind=asr provider=mock"));
    assert!(output.contains("probe_detail="));
    assert!(output.contains("health_status="));
    assert!(!output.contains("not-supported"));
    assert!(!output.contains("sk-"));
}
