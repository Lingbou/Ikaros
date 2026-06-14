// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    anthropic::AnthropicProvider,
    governance::{GovernedModelProvider, ModelRuntimeLimits},
    mock::MockModelProvider,
    ollama::OllamaProvider,
    openai_compatible::OpenAiCompatibleProvider,
    types::ModelProvider,
    usage::ModelUsageLedger,
};
use ikaros_core::{IkarosError, ModelConfig, RemoteProviderConfig, Result};
use std::path::PathBuf;

pub fn provider_from_config(
    config: &ModelConfig,
    provider_settings: &RemoteProviderConfig,
) -> Result<Box<dyn ModelProvider>> {
    match config.provider.to_ascii_lowercase().as_str() {
        "mock" => Ok(Box::new(MockModelProvider::new(config.model.clone()))),
        "openai-compatible" => Ok(Box::new(OpenAiCompatibleProvider::from_config(
            "openai-compatible",
            config,
            provider_settings,
        )?)),
        "anthropic" => Ok(Box::new(AnthropicProvider::from_config(
            "anthropic",
            config,
            provider_settings,
        )?)),
        "ollama" => Ok(Box::new(OllamaProvider::from_config(
            "ollama",
            config,
            provider_settings,
        )?)),
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
    Ok(Box::new(GovernedModelProvider::new(
        provider_from_config(config, provider_settings)?,
        ModelUsageLedger::new(audit_dir),
        ModelRuntimeLimits::from(config),
    )))
}
