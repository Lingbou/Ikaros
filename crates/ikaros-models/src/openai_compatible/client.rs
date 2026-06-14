// SPDX-License-Identifier: GPL-3.0-only

use crate::transport::{ModelTransport, ModelTransportDescriptor, descriptor};
use ikaros_core::{
    IkarosError, ModelConfig, RemoteProviderConfig, Result, resolve_config_secret,
    resolve_config_value,
};
use reqwest::Client;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    pub(super) name: String,
    pub(super) base_url: String,
    pub(super) model: String,
    pub(super) api_key: String,
    pub(super) max_retries: u8,
    pub(super) client: Client,
}

impl OpenAiCompatibleProvider {
    pub fn from_config(
        provider_name: impl Into<String>,
        config: &ModelConfig,
        provider_settings: &RemoteProviderConfig,
    ) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|source| {
                IkarosError::Message(format!("failed to build model client: {source}"))
            })?;
        Ok(Self {
            name: provider_name.into(),
            base_url: resolve_config_value(
                &provider_settings.base_url,
                "providers.model.base_url for OpenAI-compatible model provider",
            )?
            .trim_end_matches('/')
            .into(),
            model: resolve_config_value(&config.model, "model.default.model")?,
            api_key: provider_settings.api_key.clone(),
            max_retries: config.max_retries,
            client,
        })
    }

    pub(super) fn api_key(&self) -> Result<String> {
        resolve_config_secret(&self.api_key, "providers.model.api_key")
    }

    pub(super) fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    pub(crate) fn compatible_temperature(&self, temperature: Option<f32>) -> Option<f32> {
        temperature
    }
}

impl ModelTransport for OpenAiCompatibleProvider {
    fn transport_descriptor(&self) -> ModelTransportDescriptor {
        descriptor(
            self.name.clone(),
            self.model.clone(),
            "harness-agent-loop",
            "openai-compatible-chat-completions",
            Some(self.base_url.clone()),
            true,
            true,
        )
    }
}
