// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{
    IkarosError, RagConfig, RemoteProviderConfig, Result, redact_secrets, resolve_config_secret,
    resolve_config_value,
};
use ikaros_rag::{
    EmbeddingProvider, HashEmbeddingProvider, MockEmbeddingProvider, SparseEmbeddingProvider,
};
use ikaros_tools::{ExecutionEnv, NetworkEgressRequest, NetworkEgressResponse};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc, thread, time::Duration};

const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";

pub fn with_execution_env_embedding_provider<T>(
    config: &RagConfig,
    provider_settings: &RemoteProviderConfig,
    env: Arc<dyn ExecutionEnv>,
    f: impl FnOnce(&dyn EmbeddingProvider) -> Result<T>,
) -> Result<T> {
    match config.embedding_provider.to_ascii_lowercase().as_str() {
        "hash" => f(&HashEmbeddingProvider),
        "sparse" => f(&SparseEmbeddingProvider),
        "mock" => f(&MockEmbeddingProvider),
        "openai-compatible" => {
            let provider = EgressOpenAiCompatibleEmbeddingProvider::from_config(
                config.embedding_provider.to_string(),
                config,
                provider_settings,
                env,
            )?;
            f(&provider)
        }
        "ollama" => {
            let provider =
                EgressOllamaEmbeddingProvider::from_config(config, provider_settings, env)?;
            f(&provider)
        }
        other => Err(IkarosError::Message(format!(
            "unsupported embedding provider: {other}"
        ))),
    }
}

#[derive(Clone)]
struct EgressOpenAiCompatibleEmbeddingProvider {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    timeout: Duration,
    max_retries: u8,
    env: Arc<dyn ExecutionEnv>,
}

impl EgressOpenAiCompatibleEmbeddingProvider {
    fn from_config(
        provider_name: impl Into<String>,
        config: &RagConfig,
        provider_settings: &RemoteProviderConfig,
        env: Arc<dyn ExecutionEnv>,
    ) -> Result<Self> {
        Ok(Self {
            name: provider_name.into(),
            base_url: resolve_config_value(
                &provider_settings.base_url,
                "providers.embedding.base_url",
            )?
            .trim_end_matches('/')
            .into(),
            model: resolve_config_value(&config.embedding_model, "rag.embedding_model")?,
            api_key: provider_settings.api_key.clone(),
            timeout: Duration::from_millis(config.embedding_timeout_ms),
            max_retries: config.embedding_max_retries,
            env,
        })
    }

    fn api_key(&self) -> Result<String> {
        resolve_config_secret(&self.api_key, "providers.embedding.api_key")
    }
}

impl EmbeddingProvider for EgressOpenAiCompatibleEmbeddingProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let key = self.api_key()?;
        let body = OpenAiEmbeddingRequest {
            model: self.model.clone(),
            input: redact_secrets(text),
        };
        let url = format!("{}/embeddings", self.base_url);
        let mut headers = BTreeMap::new();
        headers.insert("authorization".into(), format!("Bearer {key}"));
        headers.insert("content-type".into(), "application/json".into());
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let request = NetworkEgressRequest {
                method: "POST".into(),
                url: url.clone(),
                headers: headers.clone(),
                body: Some(serde_json::to_string(&body)?),
                body_bytes: None,
            };
            match send_network_request_blocking(self.env.clone(), request, self.timeout) {
                Ok(response) => {
                    if !(200..=299).contains(&response.status) {
                        last_error = Some(format!(
                            "embedding provider returned HTTP {}: {}",
                            response.status,
                            redact_secrets(&response.body)
                        ));
                        continue;
                    }
                    let parsed: OpenAiEmbeddingResponse = serde_json::from_str(&response.body)
                        .map_err(|source| {
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

#[derive(Clone)]
struct EgressOllamaEmbeddingProvider {
    base_url: String,
    model: String,
    timeout: Duration,
    max_retries: u8,
    env: Arc<dyn ExecutionEnv>,
}

impl EgressOllamaEmbeddingProvider {
    fn from_config(
        config: &RagConfig,
        provider_settings: &RemoteProviderConfig,
        env: Arc<dyn ExecutionEnv>,
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
            env,
        })
    }
}

impl EmbeddingProvider for EgressOllamaEmbeddingProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let body = OllamaEmbedRequest {
            model: self.model.clone(),
            input: redact_secrets(text),
        };
        let url = format!("{}/api/embed", self.base_url);
        let mut headers = BTreeMap::new();
        headers.insert("content-type".into(), "application/json".into());
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let request = NetworkEgressRequest {
                method: "POST".into(),
                url: url.clone(),
                headers: headers.clone(),
                body: Some(serde_json::to_string(&body)?),
                body_bytes: None,
            };
            match send_network_request_blocking(self.env.clone(), request, self.timeout) {
                Ok(response) => {
                    if !(200..=299).contains(&response.status) {
                        last_error = Some(format!(
                            "ollama embedding provider returned HTTP {}: {}",
                            response.status,
                            redact_secrets(&response.body)
                        ));
                        continue;
                    }
                    let parsed: OllamaEmbedResponse = serde_json::from_str(&response.body)
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

fn send_network_request_blocking(
    env: Arc<dyn ExecutionEnv>,
    request: NetworkEgressRequest,
    _timeout: Duration,
) -> Result<NetworkEgressResponse> {
    thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|source| {
                IkarosError::Message(format!(
                    "failed to build embedding egress runtime: {source}"
                ))
            })?;
        runtime.block_on(env.send_network_request(request))
    })
    .join()
    .map_err(|_| IkarosError::Message("embedding egress thread panicked".into()))?
}

#[derive(Debug, Clone, Serialize)]
struct OpenAiEmbeddingRequest {
    model: String,
    input: String,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiEmbeddingResponse {
    data: Vec<OpenAiEmbeddingData>,
}

#[derive(Debug, Clone, Deserialize)]
struct OpenAiEmbeddingData {
    embedding: Vec<f32>,
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
