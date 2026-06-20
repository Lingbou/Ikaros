// SPDX-License-Identifier: GPL-3.0-only

use crate::EmbeddingProvider;
use ikaros_core::{IkarosError, RagConfig, RemoteProviderConfig, Result, redact_secrets};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::{thread, time::Duration};

const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";

#[derive(Debug, Clone)]
pub struct OllamaEmbeddingProvider {
    base_url: String,
    model: String,
    timeout: Duration,
    max_retries: u8,
}

impl OllamaEmbeddingProvider {
    pub fn from_config(
        config: &RagConfig,
        provider_settings: &RemoteProviderConfig,
    ) -> Result<Self> {
        let base_url = if provider_settings.base_url.trim().is_empty() {
            DEFAULT_OLLAMA_BASE_URL
        } else {
            provider_settings.base_url.trim()
        };
        if config.embedding_model.trim().is_empty() {
            return Err(IkarosError::Message(
                "rag.embedding_model must not be empty for ollama embeddings".into(),
            ));
        }
        Ok(Self {
            base_url: base_url.trim_end_matches('/').into(),
            model: config.embedding_model.clone(),
            timeout: Duration::from_millis(config.embedding_timeout_ms),
            max_retries: config.embedding_max_retries,
        })
    }
}

impl EmbeddingProvider for OllamaEmbeddingProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let body = OllamaEmbedRequest {
            model: self.model.clone(),
            input: redact_secrets(text),
        };
        let url = format!("{}/api/embed", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            match send_ollama_embed_request(url.clone(), body.clone(), self.timeout) {
                Ok(response) => {
                    if !(200..=299).contains(&response.status) {
                        last_error = Some(format!(
                            "ollama embedding provider returned HTTP {}: {}",
                            response.status,
                            redact_secrets(&response.text)
                        ));
                        continue;
                    }
                    let parsed: OllamaEmbedResponse = serde_json::from_str(&response.text)
                        .map_err(|source| {
                            IkarosError::Message(format!(
                                "failed to parse Ollama embedding response JSON: {source}"
                            ))
                        })?;
                    let vector = parsed
                        .embeddings
                        .into_iter()
                        .next()
                        .or(parsed.embedding)
                        .ok_or_else(|| {
                            IkarosError::Message(
                                "ollama embedding provider returned no vectors".into(),
                            )
                        })?;
                    if vector.is_empty() {
                        return Err(IkarosError::Message(
                            "ollama embedding provider returned an empty vector".into(),
                        ));
                    }
                    return Ok(vector);
                }
                Err(error) => {
                    last_error = Some(format!(
                        "ollama embedding request failed on attempt {attempt}: {error}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(last_error.unwrap_or_else(|| {
            "ollama embedding request failed".into()
        })))
    }
}

fn send_ollama_embed_request(
    url: String,
    body: OllamaEmbedRequest,
    timeout: Duration,
) -> Result<OllamaEmbeddingHttpResponse> {
    thread::spawn(move || {
        let client = Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|source| {
                IkarosError::Message(format!("failed to build Ollama embedding client: {source}"))
            })?;
        let response = client.post(&url).json(&body).send().map_err(|source| {
            IkarosError::Message(format!("ollama embedding request failed: {source}"))
        })?;
        let status = response.status().as_u16();
        let text = response.text().map_err(|source| {
            IkarosError::Message(format!(
                "failed to read Ollama embedding response: {source}"
            ))
        })?;
        Ok(OllamaEmbeddingHttpResponse { status, text })
    })
    .join()
    .map_err(|_| IkarosError::Message("ollama embedding request thread panicked".into()))?
}

struct OllamaEmbeddingHttpResponse {
    status: u16,
    text: String,
}

#[derive(Debug, Clone, Serialize)]
struct OllamaEmbedRequest {
    model: String,
    input: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OllamaEmbedResponse {
    #[serde(default)]
    embeddings: Vec<Vec<f32>>,
    #[serde(default)]
    embedding: Option<Vec<f32>>,
}

#[cfg(test)]
pub(crate) fn test_ollama_embedding_request_body(
    config: &RagConfig,
    input: &str,
) -> serde_json::Value {
    let body = OllamaEmbedRequest {
        model: config.embedding_model.clone(),
        input: redact_secrets(input),
    };
    serde_json::to_value(body).expect("serialize test Ollama embedding body")
}
