// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosConfig, ModelConfig};
use ikaros_models::{
    GovernedModelProvider, ModelProvider, ModelRequest, ModelRuntimeLimits, ModelUsageLedger,
    OpenAiCompatibleProvider,
};
use std::{
    env,
    path::{Path, PathBuf},
};

#[tokio::test]
#[ignore = "requires IKAROS_RUN_LIVE_MODEL_TESTS=1 and provider API credentials"]
async fn moonshot_live_generate_smoke() {
    let Some(live) = live_provider_config("moonshot", "https://api.moonshot.cn/v1", "kimi-k2.6")
    else {
        return;
    };
    let config = ModelConfig {
        provider: "moonshot".into(),
        runtime: "harness-agent-loop".into(),
        transport: "openai-compatible-chat-completions".into(),
        base_url: live.base_url,
        api_key: live.api_key,
        model: live.model,
        timeout_ms: 30_000,
        max_retries: 1,
        rate_limit_per_minute: Some(10),
        daily_token_budget: Some(100_000),
    };
    live_generate_smoke("moonshot", config).await;
}

#[tokio::test]
#[ignore = "requires IKAROS_RUN_LIVE_MODEL_TESTS=1 and provider API credentials"]
async fn siliconflow_live_generate_smoke() {
    let Some(live) = live_provider_config(
        "siliconflow",
        "https://api.siliconflow.cn/v1",
        "Qwen/Qwen3-Coder-30B-A3B-Instruct",
    ) else {
        return;
    };
    let config = ModelConfig {
        provider: "siliconflow".into(),
        runtime: "harness-agent-loop".into(),
        transport: "openai-compatible-chat-completions".into(),
        base_url: live.base_url,
        api_key: live.api_key,
        model: live.model,
        timeout_ms: 30_000,
        max_retries: 1,
        rate_limit_per_minute: Some(10),
        daily_token_budget: Some(100_000),
    };
    live_generate_smoke("siliconflow", config).await;
}

fn live_model_tests_enabled() -> bool {
    env::var("IKAROS_RUN_LIVE_MODEL_TESTS").ok().as_deref() == Some("1")
}

struct LiveProviderConfig {
    api_key: String,
    base_url: String,
    model: String,
}

fn live_provider_config(
    provider: &str,
    default_base_url: &str,
    default_model: &str,
) -> Option<LiveProviderConfig> {
    if !live_model_tests_enabled() {
        return None;
    }

    let local_config = load_local_config()
        .filter(|config| model_config_matches(&config.model.default, provider, default_base_url));
    let config = local_config?;
    let api_key = non_empty_config_value(&config.model.default.api_key)?;
    let base_url = non_empty_config_value(&config.model.default.base_url)
        .unwrap_or_else(|| default_base_url.into());
    let model =
        non_empty_config_value(&config.model.default.model).unwrap_or_else(|| default_model.into());

    Some(LiveProviderConfig {
        api_key,
        base_url,
        model,
    })
}

fn load_local_config() -> Option<IkarosConfig> {
    IkarosConfig::load(&default_config_path()?).ok()
}

fn default_config_path() -> Option<PathBuf> {
    if let Some(home) = env::var_os("IKAROS_HOME") {
        return Some(PathBuf::from(home).join("config.toml"));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".ikaros/config.toml"))
}

fn model_config_matches(config: &ModelConfig, provider: &str, default_base_url: &str) -> bool {
    let configured_provider = config.provider.to_ascii_lowercase();
    configured_provider == provider
        || (configured_provider == "openai-compatible"
            && config
                .base_url
                .to_ascii_lowercase()
                .contains(default_base_url.trim_start_matches("https://")))
}

fn non_empty_config_value(value: &str) -> Option<String> {
    Some(value.trim().to_owned()).filter(|value| !value.is_empty())
}

async fn live_generate_smoke(provider_name: &str, config: ModelConfig) {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = OpenAiCompatibleProvider::from_config(provider_name, &config).expect("provider");
    let governed = GovernedModelProvider::new(
        Box::new(provider),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::from(&config),
    );
    let request = ModelRequest {
        messages: vec![ikaros_models::ModelMessage::user(
            "Reply with IKAROS_OK only.",
        )],
        max_tokens: Some(128),
        temperature: Some(0.0),
        tools: Vec::new(),
    };

    let response = governed.generate(request).await.expect("live generate");
    assert_eq!(response.provider, provider_name);
    assert!(!response.content.trim().is_empty());

    assert_prompt_free_usage_log(governed.ledger().path());
}

fn assert_prompt_free_usage_log(path: &Path) {
    let raw = std::fs::read_to_string(path).expect("usage log");
    assert!(!raw.contains("Reply with"));
    assert!(!raw.contains("IKAROS_OK"));
    let records = ModelUsageLedger::from_file(path)
        .read_all()
        .expect("records");
    assert_eq!(records.len(), 1);
    assert!(records[0].total_tokens > 0);
}
