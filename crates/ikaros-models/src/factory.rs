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
use ikaros_core::{IkarosError, ModelConfig, Result};
use std::path::PathBuf;

pub fn provider_from_config(config: &ModelConfig) -> Result<Box<dyn ModelProvider>> {
    match config.provider.to_ascii_lowercase().as_str() {
        "mock" => Ok(Box::new(MockModelProvider::new(config.model.clone()))),
        "moonshot" => Ok(Box::new(OpenAiCompatibleProvider::from_config(
            "moonshot", config,
        )?)),
        "siliconflow" | "silicon-flow" => Ok(Box::new(OpenAiCompatibleProvider::from_config(
            "siliconflow",
            config,
        )?)),
        "openai-compatible" | "openai_compatible" | "openai" => Ok(Box::new(
            OpenAiCompatibleProvider::from_config("openai-compatible", config)?,
        )),
        "anthropic" | "claude" => Ok(Box::new(AnthropicProvider::from_config(
            "anthropic",
            config,
        )?)),
        "ollama" | "local-llm" | "local_llm" => {
            Ok(Box::new(OllamaProvider::from_config("ollama", config)?))
        }
        other => Err(IkarosError::Message(format!(
            "unsupported model provider: {other}"
        ))),
    }
}

pub fn governed_provider_from_config(
    config: &ModelConfig,
    audit_dir: impl Into<PathBuf>,
) -> Result<Box<dyn ModelProvider>> {
    Ok(Box::new(GovernedModelProvider::new(
        provider_from_config(config)?,
        ModelUsageLedger::new(audit_dir),
        ModelRuntimeLimits::from(config),
    )))
}
