// SPDX-License-Identifier: GPL-3.0-only
//! Model provider adapters for Ikaros.

mod anthropic;
mod factory;
mod governance;
mod http;
mod mock;
mod ollama;
mod openai_compatible;
mod params;
mod registry;
mod transport;
mod types;
mod usage;

pub use anthropic::AnthropicProvider;
pub use factory::{
    governed_provider_from_config, governed_provider_from_config_with_http_client,
    provider_from_config, provider_from_config_with_http_client,
};
pub use governance::{
    FallbackModelProvider, GovernedModelProvider, ModelRuntimeLimits, ProviderCooldownPolicy,
    ProviderRetryPolicy,
};
pub use http::{ModelHttpClient, ModelHttpRequest, ModelHttpResponse, ReqwestModelHttpClient};
pub use mock::MockModelProvider;
pub use ollama::OllamaProvider;
pub use openai_compatible::OpenAiCompatibleProvider;
pub use registry::ProviderRegistry;
pub use transport::{
    ModelTransport, ModelTransportDescriptor, model_transport_descriptor_from_config,
};
pub use types::{
    ModelContextProfile, ModelMessage, ModelProvider, ModelProviderCapabilities, ModelProviderCost,
    ModelProviderDescriptor, ModelRequest, ModelRequestDiagnostic, ModelRequestOptions,
    ModelResponse, ModelStream, ModelStreamEvent, ModelTokenizerKind, ModelToolCall,
    ModelToolDefinition, ProviderErrorKind, ProviderHealthState, ProviderHealthStatus,
    ReasoningConfig, ReasoningEffort, TokenUsage,
};
pub use usage::{ModelUsageLedger, ModelUsageRecord, ProviderHealthLedger, ProviderHealthRecord};

#[cfg(test)]
mod tests;
