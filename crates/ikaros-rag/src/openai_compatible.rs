// SPDX-License-Identifier: GPL-3.0-only

use crate::EmbeddingProvider;
use ikaros_core::{
    IkarosError, RagConfig, Result, redact_secrets, resolve_config_secret, resolve_config_value,
};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{thread, time::Duration};

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleEmbeddingProvider {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    timeout: Duration,
    max_retries: u8,
}

impl OpenAiCompatibleEmbeddingProvider {
    pub fn from_config(provider_name: impl Into<String>, config: &RagConfig) -> Result<Self> {
        Ok(Self {
            name: provider_name.into(),
            base_url: resolve_config_value(
                &config.embedding_base_url,
                "providers.embedding.base_url",
            )?
            .trim_end_matches('/')
            .into(),
            model: resolve_config_value(&config.embedding_model, "rag.embedding_model")?,
            api_key: config.embedding_api_key.clone(),
            timeout: Duration::from_millis(config.embedding_timeout_ms),
            max_retries: config.embedding_max_retries,
        })
    }

    fn api_key(&self) -> Result<String> {
        resolve_config_secret(&self.api_key, "providers.embedding.api_key")
    }
}

impl EmbeddingProvider for OpenAiCompatibleEmbeddingProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let key = self.api_key()?;
        let body = EmbeddingRequest {
            model: self.model.clone(),
            input: redact_secrets(text),
        };
        let url = format!("{}/embeddings", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result =
                send_embedding_request(url.clone(), key.clone(), body.clone(), self.timeout);
            match result {
                Ok(response) => {
                    if !(200..=299).contains(&response.status) {
                        last_error = Some(format!(
                            "embedding provider returned HTTP {}: {}",
                            response.status,
                            redact_secrets(&response.text)
                        ));
                        continue;
                    }
                    let parsed: EmbeddingResponse =
                        serde_json::from_str(&response.text).map_err(|source| {
                            IkarosError::Message(format!(
                                "failed to parse embedding response JSON: {source}"
                            ))
                        })?;
                    let Some(first) = parsed.data.into_iter().next() else {
                        return Err(IkarosError::Message(
                            "embedding provider returned no vectors".into(),
                        ));
                    };
                    if first.embedding.is_empty() {
                        return Err(IkarosError::Message(
                            "embedding provider returned an empty vector".into(),
                        ));
                    }
                    return Ok(first.embedding);
                }
                Err(source) => {
                    last_error = Some(format!(
                        "embedding request failed on attempt {attempt}: {source}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(
            last_error.unwrap_or_else(|| "embedding request failed".into()),
        ))
    }
}

fn send_embedding_request(
    url: String,
    key: String,
    body: EmbeddingRequest,
    timeout: Duration,
) -> Result<EmbeddingHttpResponse> {
    thread::spawn(move || {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|source| {
                IkarosError::Message(format!("failed to build embedding client: {source}"))
            })?;
        let response = client.post(&url).bearer_auth(&key).json(&body).send();
        let response = response.map_err(|source| {
            IkarosError::Message(format!("embedding request failed: {source}"))
        })?;
        let status = response.status().as_u16();
        let text = response.text().map_err(|source| {
            IkarosError::Message(format!("failed to read embedding response: {source}"))
        })?;
        Ok(EmbeddingHttpResponse { status, text })
    })
    .join()
    .map_err(|_| IkarosError::Message("embedding request thread panicked".into()))?
}

struct EmbeddingHttpResponse {
    status: u16,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct EmbeddingRequest {
    model: String,
    input: String,
}

#[derive(Debug, Clone, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Clone, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[cfg(test)]
pub(crate) fn test_embedding_request_body(config: &RagConfig, input: &str) -> serde_json::Value {
    let body = EmbeddingRequest {
        model: config.embedding_model.clone(),
        input: redact_secrets(input),
    };
    serde_json::to_value(body).expect("serialize test embedding body")
}
