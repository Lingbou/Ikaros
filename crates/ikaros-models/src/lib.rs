// SPDX-License-Identifier: GPL-3.0-only
//! Model provider adapters for Ikaros.

mod anthropic;
mod factory;
mod governance;
mod mock;
mod ollama;
mod openai_compatible;
mod params;
mod transport;
mod types;
mod usage;

pub use anthropic::AnthropicProvider;
pub use factory::{governed_provider_from_config, provider_from_config};
pub use governance::{GovernedModelProvider, ModelRuntimeLimits};
pub use mock::MockModelProvider;
pub use ollama::OllamaProvider;
pub use openai_compatible::OpenAiCompatibleProvider;
pub use transport::{
    ModelTransport, ModelTransportDescriptor, model_transport_descriptor_from_config,
};
pub use types::{
    ModelMessage, ModelProvider, ModelRequest, ModelRequestDiagnostic, ModelRequestOptions,
    ModelResponse, ModelStream, ModelStreamEvent, ModelToolCall, ModelToolDefinition,
    ReasoningConfig, ReasoningEffort, TokenUsage,
};
pub use usage::{ModelUsageLedger, ModelUsageRecord};

#[cfg(test)]
mod tests;
