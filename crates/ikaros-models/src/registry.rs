// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    openai_compatible::{ProviderProfile, ReasoningPolicy},
    types::{
        ModelContextProfile, ModelProviderCapabilities, ModelProviderCost, ModelProviderDescriptor,
        ModelProviderProfileCatalogEntry, ModelProviderProfilePolicy, ModelTokenizerKind,
        ProviderHealthState,
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
        self.descriptor_with_profile(provider, base_url, model, "auto")
    }

    pub fn descriptor_with_profile(
        &self,
        provider: &str,
        base_url: &str,
        model: &str,
        compat_profile: &str,
    ) -> Result<ModelProviderDescriptor> {
        let provider = provider.trim().to_ascii_lowercase();
        match provider.as_str() {
            "mock" => Ok(mock_descriptor(model)),
            "openai-compatible" => {
                let profile = ProviderProfile::resolve_configured(compat_profile, base_url, model)?;
                Ok(ModelProviderDescriptor {
                    provider,
                    model: model.into(),
                    profile: profile.id.into(),
                    profile_policy: openai_compatible_profile_policy(&profile),
                    capabilities: openai_compatible_capabilities(&profile),
                    context: profile.context,
                    cost: unknown_cost(),
                    health: ProviderHealthState::new("openai-compatible", model),
                })
            }
            "anthropic" => Ok(ModelProviderDescriptor {
                provider,
                model: model.into(),
                profile: "anthropic-compatible".into(),
                profile_policy: ModelProviderProfilePolicy::native("anthropic-native")
                    .with_prompt_cache("anthropic-system-prefix-ephemeral"),
                capabilities: ModelProviderCapabilities {
                    chat: true,
                    streaming: false,
                    tool_calls: true,
                    reasoning: true,
                    json_mode: false,
                    network: true,
                    image_input: true,
                    audio_input: false,
                    file_input: false,
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
                profile_policy: ModelProviderProfilePolicy::native("ollama-native"),
                capabilities: ModelProviderCapabilities {
                    chat: true,
                    streaming: true,
                    tool_calls: true,
                    reasoning: false,
                    json_mode: false,
                    network: false,
                    image_input: true,
                    audio_input: false,
                    file_input: false,
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

    pub fn openai_compatible_profile_catalog(&self) -> Vec<ModelProviderProfileCatalogEntry> {
        ProviderProfile::catalog()
            .iter()
            .map(|spec| {
                let profile = ProviderProfile::resolve_spec(
                    spec,
                    spec.auto_base_url_markers
                        .first()
                        .copied()
                        .unwrap_or_default(),
                    spec.auto_model_tail_prefixes
                        .first()
                        .copied()
                        .unwrap_or("catalog-128k"),
                );
                ModelProviderProfileCatalogEntry {
                    provider: "openai-compatible".into(),
                    profile: profile.id.into(),
                    profile_policy: openai_compatible_profile_policy(&profile),
                    capabilities: openai_compatible_capabilities(&profile),
                    context: profile.context,
                    auto_base_url_markers: spec
                        .auto_base_url_markers
                        .iter()
                        .map(|marker| (*marker).to_owned())
                        .collect(),
                    auto_model_markers: spec
                        .auto_model_markers
                        .iter()
                        .map(|marker| (*marker).to_owned())
                        .collect(),
                    auto_model_tail_prefixes: spec
                        .auto_model_tail_prefixes
                        .iter()
                        .map(|prefix| (*prefix).to_owned())
                        .collect(),
                }
            })
            .collect()
    }
}

fn mock_descriptor(model: &str) -> ModelProviderDescriptor {
    ModelProviderDescriptor {
        provider: "mock".into(),
        model: model.into(),
        profile: "mock".into(),
        profile_policy: ModelProviderProfilePolicy::native("mock"),
        capabilities: ModelProviderCapabilities {
            chat: true,
            streaming: true,
            tool_calls: true,
            reasoning: false,
            json_mode: true,
            network: false,
            image_input: true,
            audio_input: true,
            file_input: true,
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

fn openai_compatible_profile_policy(profile: &ProviderProfile) -> ModelProviderProfilePolicy {
    ModelProviderProfilePolicy {
        temperature: profile.temperature_policy.as_str().into(),
        reasoning: profile.reasoning_policy.as_str().into(),
        message: profile.message_policy.as_str().into(),
        tool_schema: profile.tool_schema_policy.as_str().into(),
        request_body: profile.request_body_policy.as_str().into(),
        prompt_cache: openai_compatible_prompt_cache_policy(profile).into(),
        retry_without_parameters: profile
            .retry_without_parameters
            .iter()
            .map(|parameter| (*parameter).to_owned())
            .collect(),
    }
}

fn openai_compatible_prompt_cache_policy(profile: &ProviderProfile) -> &'static str {
    if profile.message_policy.as_str() == "qwen-text-parts-system-cache" {
        "qwen-system-ephemeral"
    } else {
        "none"
    }
}

fn openai_compatible_capabilities(profile: &ProviderProfile) -> ModelProviderCapabilities {
    ModelProviderCapabilities {
        chat: true,
        streaming: true,
        tool_calls: true,
        reasoning: !matches!(profile.reasoning_policy, ReasoningPolicy::None),
        json_mode: true,
        network: profile.network_access,
        image_input: true,
        audio_input: true,
        file_input: true,
    }
}

fn unknown_cost() -> ModelProviderCost {
    ModelProviderCost {
        currency: "USD".into(),
        input_per_million: None,
        output_per_million: None,
        cache_read_per_million: None,
        cache_write_per_million: None,
    }
}

fn free_local_cost() -> ModelProviderCost {
    ModelProviderCost {
        currency: "USD".into(),
        input_per_million: None,
        output_per_million: None,
        cache_read_per_million: None,
        cache_write_per_million: None,
    }
}
