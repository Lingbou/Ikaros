// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    anthropic::AnthropicProvider,
    governance::{GovernedModelProvider, ModelRuntimeLimits, ProviderRetryPolicy},
    http::ModelHttpClient,
    mock::MockModelProvider,
    ollama::OllamaProvider,
    openai_compatible::OpenAiCompatibleProvider,
    types::ModelProvider,
    usage::ModelUsageLedger,
};
use ikaros_core::{IkarosError, ModelConfig, RemoteProviderConfig, Result};
use std::{path::PathBuf, sync::Arc};

pub fn provider_from_config(
    config: &ModelConfig,
    provider_settings: &RemoteProviderConfig,
) -> Result<Box<dyn ModelProvider>> {
    provider_from_config_with_http_client(config, provider_settings, None)
}

pub fn provider_from_config_with_http_client(
    config: &ModelConfig,
    provider_settings: &RemoteProviderConfig,
    http: Option<Arc<dyn ModelHttpClient>>,
) -> Result<Box<dyn ModelProvider>> {
    match config.provider.to_ascii_lowercase().as_str() {
        "mock" => Ok(Box::new(MockModelProvider::new(config.model.clone()))),
        "openai-compatible" => {
            let provider = if let Some(http) = http {
                OpenAiCompatibleProvider::from_config_with_http_client(
                    "openai-compatible",
                    config,
                    provider_settings,
                    http,
                )?
            } else {
                OpenAiCompatibleProvider::from_config(
                    "openai-compatible",
                    config,
                    provider_settings,
                )?
            };
            Ok(Box::new(provider))
        }
        "anthropic" => {
            let provider = if let Some(http) = http {
                AnthropicProvider::from_config_with_http_client(
                    "anthropic",
                    config,
                    provider_settings,
                    http,
                )?
            } else {
                AnthropicProvider::from_config("anthropic", config, provider_settings)?
            };
            Ok(Box::new(provider))
        }
        "ollama" => {
            let provider = if let Some(http) = http {
                OllamaProvider::from_config_with_http_client(
                    "ollama",
                    config,
                    provider_settings,
                    http,
                )?
            } else {
                OllamaProvider::from_config("ollama", config, provider_settings)?
            };
            Ok(Box::new(provider))
        }
        other => Err(IkarosError::Message(format!(
            "unsupported model provider: {other}"
        ))),
    }
}

pub fn governed_provider_from_config(
    config: &ModelConfig,
    provider_settings: &RemoteProviderConfig,
    audit_dir: impl Into<PathBuf>,
) -> Result<Box<dyn ModelProvider>> {
    governed_provider_from_config_with_http_client(config, provider_settings, audit_dir, None)
}

pub fn governed_provider_from_config_with_http_client(
    config: &ModelConfig,
    provider_settings: &RemoteProviderConfig,
    audit_dir: impl Into<PathBuf>,
    http: Option<Arc<dyn ModelHttpClient>>,
) -> Result<Box<dyn ModelProvider>> {
    Ok(Box::new(governed_model_provider_from_config(
        config,
        provider_settings,
        audit_dir,
        http,
    )?))
}

pub(crate) fn governed_model_provider_from_config(
    config: &ModelConfig,
    provider_settings: &RemoteProviderConfig,
    audit_dir: impl Into<PathBuf>,
    http: Option<Arc<dyn ModelHttpClient>>,
) -> Result<GovernedModelProvider> {
    Ok(GovernedModelProvider::new_with_retry_policy(
        provider_from_config_with_http_client(config, provider_settings, http)?,
        ModelUsageLedger::new(audit_dir),
        ModelRuntimeLimits::from(config),
        ProviderRetryPolicy::from(config),
    ))
}
