// SPDX-License-Identifier: GPL-3.0-only

use super::profile::OpenAiCompatProfile;
use crate::{
    params::model_request_options_from_config,
    transport::{ModelTransport, ModelTransportDescriptor, descriptor},
    types::ModelRequestOptions,
};
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
    pub(super) profile: OpenAiCompatProfile,
    pub(super) default_options: ModelRequestOptions,
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
        let base_url = resolve_config_value(
            &provider_settings.base_url,
            "providers.model.base_url for OpenAI-compatible model provider",
        )?
        .trim_end_matches('/')
        .to_owned();
        let model = resolve_config_value(&config.model, "model.default.model")?;
        let profile = OpenAiCompatProfile::resolve(&config.compat_profile, &base_url, &model)?;
        Ok(Self {
            name: provider_name.into(),
            base_url,
            model,
            api_key: provider_settings.api_key.clone(),
            max_retries: config.max_retries,
            profile,
            default_options: model_request_options_from_config(config)?,
            client,
        })
    }

    pub(super) fn api_key(&self) -> Result<String> {
        resolve_config_secret(&self.api_key, "providers.model.api_key")
    }

    pub(super) fn chat_completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    #[cfg(test)]
    pub(crate) fn compat_profile_id(&self) -> &'static str {
        self.profile.id()
    }

    #[cfg(test)]
    pub(crate) fn test_chat_completion_body(
        &self,
        request: crate::types::ModelRequest,
        stream: bool,
    ) -> Result<serde_json::Value> {
        super::request_builder::build_chat_completion_request(
            &self.model,
            &self.base_url,
            self.profile,
            &self.default_options,
            request.redacted(),
            stream,
        )
        .map(|prepared| prepared.body)
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
