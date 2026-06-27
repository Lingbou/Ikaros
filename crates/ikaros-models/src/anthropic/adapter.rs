// SPDX-License-Identifier: GPL-3.0-only

use super::{errors, request, response, stream};
use crate::http::{ModelHttpClient, ReqwestModelHttpClient};
use crate::transport::{ModelTransport, ModelTransportDescriptor, descriptor};
use crate::types::{
    ModelContextProfile, ModelProvider, ModelProviderCapabilities, ModelRequest, ModelResponse,
    ModelStream, ModelTokenizerKind,
};
use async_trait::async_trait;
use ikaros_core::{IkarosError, ModelConfig, RemoteProviderConfig, Result, resolve_config_secret};
use std::{sync::Arc, time::Duration};

#[derive(Clone)]
pub struct AnthropicProvider {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    max_retries: u8,
    http: Arc<dyn ModelHttpClient>,
}

impl AnthropicProvider {
    pub fn from_config(
        provider_name: impl Into<String>,
        config: &ModelConfig,
        provider_settings: &RemoteProviderConfig,
    ) -> Result<Self> {
        Self::from_config_with_http_client(
            provider_name,
            config,
            provider_settings,
            Arc::new(ReqwestModelHttpClient::new(Duration::from_millis(
                config.timeout_ms,
            ))?),
        )
    }

    pub fn from_config_with_http_client(
        provider_name: impl Into<String>,
        config: &ModelConfig,
        provider_settings: &RemoteProviderConfig,
        http: Arc<dyn ModelHttpClient>,
    ) -> Result<Self> {
        Ok(Self {
            name: provider_name.into(),
            base_url: request::provider_base_url(provider_settings)?,
            model: ikaros_core::resolve_config_value(&config.model, "model.default.model")?,
            api_key: provider_settings.api_key.clone(),
            max_retries: config.max_retries,
            http,
        })
    }

    fn api_key(&self) -> Result<String> {
        resolve_config_secret(&self.api_key, "providers.model.api_key")
    }
}

impl ModelTransport for AnthropicProvider {
    fn transport_descriptor(&self) -> ModelTransportDescriptor {
        descriptor(
            self.name.clone(),
            self.model.clone(),
            "harness-agent-loop",
            "anthropic-messages",
            Some(self.base_url.clone()),
            true,
            true,
        )
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn context_profile(&self) -> ModelContextProfile {
        ModelContextProfile::new(
            request::anthropic_context_window(&self.model),
            request::anthropic_default_max_tokens(&self.model),
            ModelTokenizerKind::Anthropic,
            "anthropic",
        )
    }

    fn capabilities(&self) -> ModelProviderCapabilities {
        ModelProviderCapabilities {
            chat: true,
            streaming: false,
            tool_calls: true,
            reasoning: true,
            json_mode: false,
            network: true,
            image_input: true,
            audio_input: false,
            file_input: false,
        }
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let key = self.api_key()?;
        let request = request.redacted();
        let body = request::anthropic_messages_request_body(&self.model, request);
        let url = format!("{}/messages", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result = self
                .http
                .send(request::anthropic_http_post(&url, &key, &body)?)
                .await;
            match result {
                Ok(response) => {
                    let status = response.status;
                    let text = response.body;
                    if !(200..=299).contains(&status) {
                        last_error = Some(errors::provider_http_error(status, &text));
                        continue;
                    }
                    return response::parse_messages_response(&text, &self.name, &self.model);
                }
                Err(source) => {
                    last_error = Some(format!(
                        "Anthropic model request failed on attempt {attempt}: {source}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(
            last_error.unwrap_or_else(|| "Anthropic model request failed".into()),
        ))
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let key = self.api_key()?;
        let request = request.redacted();
        let mut body = request::anthropic_messages_request_body(&self.model, request);
        body.stream = Some(true);
        let url = format!("{}/messages", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result = self
                .http
                .send(request::anthropic_http_post(&url, &key, &body)?)
                .await;
            match result {
                Ok(response) => {
                    let status = response.status;
                    let text = response.body;
                    if !(200..=299).contains(&status) {
                        last_error = Some(errors::stream_http_error(status, &text));
                        continue;
                    }
                    return stream::parse_stream_response(&text, &self.name, &self.model);
                }
                Err(source) => {
                    last_error = Some(format!(
                        "Anthropic model stream failed on attempt {attempt}: {source}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(
            last_error.unwrap_or_else(|| "Anthropic model stream failed".into()),
        ))
    }
}
