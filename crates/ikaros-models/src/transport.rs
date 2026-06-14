// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{ModelConfig, RemoteProviderConfig};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelTransportDescriptor {
    pub provider: String,
    pub model: String,
    pub runtime: String,
    pub transport: String,
    pub base_url: Option<String>,
    pub supports_streaming: bool,
    pub normalizes_tool_calls: bool,
}

pub trait ModelTransport: Send + Sync {
    fn transport_descriptor(&self) -> ModelTransportDescriptor;
}

pub fn model_transport_descriptor_from_config(
    config: &ModelConfig,
    provider_settings: &RemoteProviderConfig,
) -> ModelTransportDescriptor {
    ModelTransportDescriptor {
        provider: config.provider.clone(),
        model: config.model.clone(),
        runtime: normalized_or(&config.runtime, "harness-agent-loop"),
        transport: normalized_or(
            &config.transport,
            default_transport_for_provider(&config.provider),
        ),
        base_url: non_empty(provider_settings.base_url.clone()),
        supports_streaming: true,
        normalizes_tool_calls: true,
    }
}

pub(crate) fn descriptor(
    provider: impl Into<String>,
    model: impl Into<String>,
    runtime: impl Into<String>,
    transport: impl Into<String>,
    base_url: Option<String>,
    supports_streaming: bool,
    normalizes_tool_calls: bool,
) -> ModelTransportDescriptor {
    ModelTransportDescriptor {
        provider: provider.into(),
        model: model.into(),
        runtime: runtime.into(),
        transport: transport.into(),
        base_url,
        supports_streaming,
        normalizes_tool_calls,
    }
}

fn default_transport_for_provider(provider: &str) -> &'static str {
    match provider {
        "openai-compatible" => "openai-compatible-chat-completions",
        "anthropic" => "anthropic-messages",
        "ollama" => "ollama-chat",
        "mock" => "mock",
        _ => "",
    }
}

fn normalized_or(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}
