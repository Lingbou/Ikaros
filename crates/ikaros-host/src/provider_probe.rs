// SPDX-License-Identifier: GPL-3.0-only

use crate::builder::runtime_execution_env;
use crate::model_http::EgressModelHttpClient;
use ikaros_core::{
    IkarosConfig, IkarosError, IkarosPaths, ModelConfig, RemoteProviderConfig, Result,
    redact_secrets, resolve_config_secret, resolve_config_value,
};
use ikaros_execution::harness::NetworkEgressRequest;
use ikaros_providers::model::{ModelRequest, governed_provider_from_config_with_http_client};
use ikaros_state::rag::{LocalRagStore, RagQuery};
use serde_json::Value;
use std::{collections::BTreeMap, path::Path, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProviderLiveProbeReport {
    pub provider: String,
    pub model: String,
    pub usage_total: u32,
}

pub async fn model_provider_live_probe(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
    model_config: &ModelConfig,
    model_provider: &RemoteProviderConfig,
    prompt: impl Into<String>,
) -> Result<ModelProviderLiveProbeReport> {
    let env = runtime_execution_env(config, workspace)?;
    let provider = governed_provider_from_config_with_http_client(
        model_config,
        model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(env))),
    )?;
    let response = provider
        .generate(ModelRequest::from_user_text(prompt.into()))
        .await?;
    Ok(ModelProviderLiveProbeReport {
        provider: response.provider,
        model: response.model,
        usage_total: response.usage.total_or_prompt_completion(),
    })
}

pub async fn embedding_provider_live_probe(
    paths: &IkarosPaths,
    workspace: &Path,
    config: &IkarosConfig,
) -> Result<String> {
    if matches!(
        config.rag.embedding_provider.to_ascii_lowercase().as_str(),
        "openai-compatible" | "ollama"
    ) {
        return remote_embedding_live_probe(workspace, config).await;
    }
    let store = LocalRagStore::new(&paths.rag_dir, &config.rag.backend)?;
    let hits = store.search_with_embedding_provider(
        RagQuery {
            query: "ikaros live embedding probe".into(),
            top_k: 1,
            scope: None,
        },
        &config.rag.embedding_provider,
    )?;
    Ok(format!("hits={}", hits.len()))
}

async fn remote_embedding_live_probe(workspace: &Path, config: &IkarosConfig) -> Result<String> {
    let env = runtime_execution_env(config, workspace)?;
    let provider = config.rag.embedding_provider.to_ascii_lowercase();
    let response = match provider.as_str() {
        "openai-compatible" => {
            let base_url = resolve_config_value(
                &config.providers.embedding.base_url,
                "providers.embedding.base_url",
            )?
            .trim_end_matches('/')
            .to_owned();
            let key = resolve_config_secret(
                &config.providers.embedding.api_key,
                "providers.embedding.api_key",
            )?;
            let mut headers = BTreeMap::new();
            headers.insert("authorization".into(), format!("Bearer {key}"));
            headers.insert("content-type".into(), "application/json".into());
            env.send_network_request(NetworkEgressRequest {
                method: "POST".into(),
                url: format!("{base_url}/embeddings"),
                headers,
                body: Some(
                    serde_json::json!({
                        "model": &config.rag.embedding_model,
                        "input": "ikaros live embedding probe"
                    })
                    .to_string(),
                ),
                body_bytes: None,
            })
            .await?
        }
        "ollama" => {
            let base_url = if config.providers.embedding.base_url.trim().is_empty() {
                "http://127.0.0.1:11434".to_owned()
            } else {
                config
                    .providers
                    .embedding
                    .base_url
                    .trim_end_matches('/')
                    .to_owned()
            };
            let mut headers = BTreeMap::new();
            headers.insert("content-type".into(), "application/json".into());
            env.send_network_request(NetworkEgressRequest {
                method: "POST".into(),
                url: format!("{base_url}/api/embed"),
                headers,
                body: Some(
                    serde_json::json!({
                        "model": &config.rag.embedding_model,
                        "input": "ikaros live embedding probe"
                    })
                    .to_string(),
                ),
                body_bytes: None,
            })
            .await?
        }
        _ => unreachable!("remote embedding provider is prefiltered"),
    };
    if (200..=299).contains(&response.status) {
        let vector_count = embedding_vector_count(&response.body);
        return Ok(format!("vectors={vector_count}"));
    }
    Err(IkarosError::Message(format!(
        "http_status={} body={}",
        response.status,
        redact_secrets(&response.body)
    )))
}

fn embedding_vector_count(body: &str) -> usize {
    let Ok(value) = serde_json::from_str::<Value>(body) else {
        return 0;
    };
    if let Some(data) = value.get("data").and_then(Value::as_array) {
        return data
            .iter()
            .filter(|item| item.get("embedding").is_some_and(embedding_value_non_empty))
            .count();
    }
    if let Some(embeddings) = value.get("embeddings").and_then(Value::as_array) {
        return embeddings.len();
    }
    value
        .get("embedding")
        .map(|embedding| usize::from(embedding_value_non_empty(embedding)))
        .unwrap_or(0)
}

fn embedding_value_non_empty(value: &Value) -> bool {
    value
        .as_array()
        .is_some_and(|embedding| !embedding.is_empty())
        || value
            .as_str()
            .is_some_and(|embedding| !embedding.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_matrix_counts_float_and_base64_embedding_vectors() {
        assert_eq!(
            embedding_vector_count(
                r#"{"data":[{"embedding":[0.1,0.2]},{"embedding":"AACAPwAAIMA="}]}"#
            ),
            2
        );
        assert_eq!(
            embedding_vector_count(r#"{"embeddings":[[0.1,0.2],"AACAPwAAIMA="]}"#),
            2
        );
        assert_eq!(embedding_vector_count(r#"{"embedding":"AACAPwAAIMA="}"#), 1);
    }
}
