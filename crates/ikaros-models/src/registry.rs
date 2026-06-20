// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    openai_compatible::OpenAiCompatProfile,
    types::{
        ModelContextProfile, ModelProviderCapabilities, ModelProviderCost, ModelProviderDescriptor,
        ModelTokenizerKind, ProviderHealthState,
    },
};
use ikaros_core::{IkarosError, Result};

#[derive(Debug, Clone, Default)]
pub struct ProviderRegistry;

impl ProviderRegistry {
    pub fn descriptor(
        &self,
        provider: &str,
        base_url: &str,
        model: &str,
    ) -> Result<ModelProviderDescriptor> {
        let provider = provider.trim().to_ascii_lowercase();
        match provider.as_str() {
            "mock" => Ok(mock_descriptor(model)),
            "openai-compatible" => {
                let profile = OpenAiCompatProfile::resolve("auto", base_url, model)?;
                Ok(ModelProviderDescriptor {
                    provider,
                    model: model.into(),
                    profile: profile.id().into(),
                    capabilities: openai_compatible_capabilities(profile),
                    context: profile.context_profile(model),
                    cost: unknown_cost(),
                    health: ProviderHealthState::new("openai-compatible", model),
                })
            }
            "anthropic" => Ok(ModelProviderDescriptor {
                provider,
                model: model.into(),
                profile: "anthropic-compatible".into(),
                capabilities: ModelProviderCapabilities {
                    chat: true,
                    streaming: false,
                    tool_calls: true,
                    reasoning: true,
                    json_mode: false,
                    network: true,
                },
                context: ModelContextProfile::new(
                    200_000,
                    4_096,
                    ModelTokenizerKind::Anthropic,
                    "provider-registry:anthropic",
                ),
                cost: unknown_cost(),
                health: ProviderHealthState::new("anthropic", model),
            }),
            "ollama" => Ok(ModelProviderDescriptor {
                provider,
                model: model.into(),
                profile: "ollama-local".into(),
                capabilities: ModelProviderCapabilities {
                    chat: true,
                    streaming: true,
                    tool_calls: true,
                    reasoning: false,
                    json_mode: false,
                    network: false,
                },
                context: ModelContextProfile::new(
                    32_768,
                    2_048,
                    ModelTokenizerKind::Ollama,
                    "provider-registry:ollama",
                ),
                cost: free_local_cost(),
                health: ProviderHealthState::new("ollama", model),
            }),
            other => Err(IkarosError::Message(format!(
                "unsupported model provider: {other}"
            ))),
        }
    }
}

fn mock_descriptor(model: &str) -> ModelProviderDescriptor {
    ModelProviderDescriptor {
        provider: "mock".into(),
        model: model.into(),
        profile: "mock".into(),
        capabilities: ModelProviderCapabilities {
            chat: true,
            streaming: true,
            tool_calls: true,
            reasoning: false,
            json_mode: true,
            network: false,
        },
        context: ModelContextProfile::new(
            8_192,
            1_024,
            ModelTokenizerKind::Mock,
            "provider-registry:mock",
        ),
        cost: free_local_cost(),
        health: ProviderHealthState::new("mock", model),
    }
}

fn openai_compatible_capabilities(profile: OpenAiCompatProfile) -> ModelProviderCapabilities {
    ModelProviderCapabilities {
        chat: true,
        streaming: true,
        tool_calls: true,
        reasoning: matches!(
            profile,
            OpenAiCompatProfile::MoonshotKimi
                | OpenAiCompatProfile::DeepSeek
                | OpenAiCompatProfile::GeminiOpenAi
                | OpenAiCompatProfile::OpenRouter
        ),
        json_mode: true,
        network: !matches!(profile, OpenAiCompatProfile::LocalOpenAiCompatible),
    }
}

fn unknown_cost() -> ModelProviderCost {
    ModelProviderCost {
        currency: "USD".into(),
        input_per_million: None,
        output_per_million: None,
    }
}

fn free_local_cost() -> ModelProviderCost {
    ModelProviderCost {
        currency: "USD".into(),
        input_per_million: None,
        output_per_million: None,
    }
}
