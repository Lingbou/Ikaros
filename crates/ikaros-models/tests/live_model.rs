// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosConfig, ModelConfig, RemoteProviderConfig};
use ikaros_models::{
    GovernedModelProvider, ModelProvider, ModelRequest, ModelRequestOptions, ModelRuntimeLimits,
    ModelUsageLedger, OpenAiCompatibleProvider,
};
use std::{
    env,
    path::{Path, PathBuf},
};

#[tokio::test]
#[ignore = "requires IKAROS_RUN_LIVE_MODEL_TESTS=1 and provider API credentials"]
async fn openai_compatible_live_generate_smoke() {
    let Some(live) = live_provider_config() else {
        return;
    };
    let config = ModelConfig {
        provider: "openai-compatible".into(),
        runtime: "harness-agent-loop".into(),
        transport: "openai-compatible-chat-completions".into(),
        model: live.model,
        timeout_ms: 30_000,
        max_retries: 1,
        rate_limit_per_minute: Some(10),
        daily_token_budget: Some(100_000),
        ..ModelConfig::default()
    };
    live_generate_smoke(config, live.provider_settings).await;
}

fn live_model_tests_enabled() -> bool {
    env::var("IKAROS_RUN_LIVE_MODEL_TESTS").ok().as_deref() == Some("1")
}

struct LiveProviderConfig {
    provider_settings: RemoteProviderConfig,
    model: String,
}

fn live_provider_config() -> Option<LiveProviderConfig> {
    if !live_model_tests_enabled() {
        return None;
    }

    let local_config = load_local_config().filter(model_config_matches);
    let config = local_config?;
    let api_key = non_empty_config_value(&config.providers.model.api_key)?;
    let base_url = non_empty_config_value(&config.providers.model.base_url)?;
    let model = non_empty_config_value(&config.model.default.model)?;

    Some(LiveProviderConfig {
        provider_settings: RemoteProviderConfig { api_key, base_url },
        model,
    })
}

fn load_local_config() -> Option<IkarosConfig> {
    IkarosConfig::load(&default_config_path()?).ok()
}

fn default_config_path() -> Option<PathBuf> {
    if let Some(home) = env::var_os("IKAROS_HOME") {
        return Some(PathBuf::from(home).join("config.yaml"));
    }
    env::var_os("HOME").map(|home| PathBuf::from(home).join(".ikaros/config.yaml"))
}

fn model_config_matches(config: &IkarosConfig) -> bool {
    config.model.default.provider == "openai-compatible"
        && config.model.default.transport == "openai-compatible-chat-completions"
}

fn non_empty_config_value(value: &str) -> Option<String> {
    Some(value.trim().to_owned()).filter(|value| !value.is_empty())
}

async fn live_generate_smoke(config: ModelConfig, provider_settings: RemoteProviderConfig) {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider =
        OpenAiCompatibleProvider::from_config("openai-compatible", &config, &provider_settings)
            .expect("provider");
    let governed = GovernedModelProvider::new(
        Box::new(provider),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::from(&config),
    );
    let request = ModelRequest {
        messages: vec![ikaros_models::ModelMessage::user(
            "Reply with IKAROS_OK only.",
        )],
        options: ModelRequestOptions {
            max_tokens: Some(128),
            temperature: Some(0.0),
            ..ModelRequestOptions::default()
        },
        tools: Vec::new(),
    };

    let response = governed.generate(request).await.expect("live generate");
    assert_eq!(response.provider, "openai-compatible");
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
