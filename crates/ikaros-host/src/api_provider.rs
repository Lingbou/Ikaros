// SPDX-License-Identifier: GPL-3.0-only

use crate::builder::{host_agent_context_shape_checked, services_for_instance};
use crate::model_http::EgressModelHttpClient;
use ikaros_core::{
    AgentInstance, IkarosConfig, IkarosError, IkarosPaths, RagConfig, RemoteProviderConfig, Result,
};
use ikaros_execution::harness::{NetworkEgressRequest, NetworkEgressResponse};
use ikaros_providers::model::{ModelHttpClient, ModelHttpRequest, ModelHttpResponse};
use ikaros_skills::with_execution_env_embedding_provider;
use serde_json::Value;
use std::{collections::BTreeMap, path::Path};

pub struct ApiEmbeddingServices {
    pub config: IkarosConfig,
    pub agent_instance: AgentInstance,
    pub rag_config: RagConfig,
    pub vectors: Vec<Vec<f32>>,
}

pub fn api_embedding_services_shape_checked(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    embedding_model_override: Option<&str>,
    inputs: &[String],
) -> Result<ApiEmbeddingServices> {
    let host = host_agent_context_shape_checked(paths, workspace, agent_override)?;
    let mut rag_config = host.config.rag.clone();
    if let Some(model) = embedding_model_override
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        rag_config.embedding_model = model.to_owned();
    }
    let services = services_for_instance(paths, &host.config, &host.agent_instance)?;
    let vectors = with_execution_env_embedding_provider(
        &rag_config,
        &host.config.providers.embedding,
        services.session.env.clone(),
        |provider| {
            inputs
                .iter()
                .map(|input| provider.embed(input))
                .collect::<Result<Vec<_>>>()
        },
    )?;
    Ok(ApiEmbeddingServices {
        config: host.config,
        agent_instance: host.agent_instance,
        rag_config,
        vectors,
    })
}

pub async fn send_provider_json_request_for_instance(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent: &AgentInstance,
    provider: &RemoteProviderConfig,
    path: &str,
    body: &Value,
) -> Result<NetworkEgressResponse> {
    let body = serde_json::to_string(body)?;
    send_provider_request_for_instance(
        paths,
        config,
        agent,
        provider,
        path,
        "application/json".into(),
        Some(body),
        None,
    )
    .await
}

pub async fn send_provider_bytes_request_for_instance(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent: &AgentInstance,
    provider: &RemoteProviderConfig,
    path: &str,
    content_type: String,
    body: Vec<u8>,
) -> Result<NetworkEgressResponse> {
    send_provider_request_for_instance(
        paths,
        config,
        agent,
        provider,
        path,
        content_type,
        None,
        Some(body),
    )
    .await
}

pub async fn send_model_json_request_for_instance(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent: &AgentInstance,
    provider: &RemoteProviderConfig,
    path: &str,
    body: &Value,
    missing_base_url_message: &str,
) -> Result<ModelHttpResponse> {
    let request = model_json_request(provider, path, body, missing_base_url_message)?;
    let services = services_for_instance(paths, config, agent)?;
    EgressModelHttpClient::new(services.session.env.clone())
        .send(request)
        .await
}

async fn send_provider_request_for_instance(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent: &AgentInstance,
    provider: &RemoteProviderConfig,
    path: &str,
    content_type: String,
    body: Option<String>,
    body_bytes: Option<Vec<u8>>,
) -> Result<NetworkEgressResponse> {
    let services = services_for_instance(paths, config, agent)?;
    let mut headers = BTreeMap::new();
    headers.insert("content-type".into(), content_type);
    if !provider.api_key.trim().is_empty() {
        headers.insert(
            "authorization".into(),
            format!("Bearer {}", provider.api_key.trim()),
        );
    }
    let request = provider_network_request(provider, path, headers, body, body_bytes)?;
    services.session.env.send_network_request(request).await
}

fn provider_network_request(
    provider: &RemoteProviderConfig,
    path: &str,
    headers: BTreeMap<String, String>,
    body: Option<String>,
    body_bytes: Option<Vec<u8>>,
) -> Result<NetworkEgressRequest> {
    let base_url = provider.base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return Err(IkarosError::Message(
            "provider base_url is required for API provider proxy routes".into(),
        ));
    }
    Ok(NetworkEgressRequest {
        method: "POST".into(),
        url: format!("{base_url}{path}"),
        headers,
        body,
        body_bytes,
    })
}

fn model_json_request(
    provider: &RemoteProviderConfig,
    path: &str,
    body: &Value,
    missing_base_url_message: &str,
) -> Result<ModelHttpRequest> {
    let base_url = provider.base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        return Err(IkarosError::Message(missing_base_url_message.into()));
    }
    let mut headers = BTreeMap::new();
    headers.insert("content-type".into(), "application/json".into());
    if !provider.api_key.trim().is_empty() {
        headers.insert(
            "authorization".into(),
            format!("Bearer {}", provider.api_key.trim()),
        );
    }
    Ok(ModelHttpRequest {
        method: "POST".into(),
        url: format!("{base_url}{path}"),
        headers,
        body: serde_json::to_string(body)?,
    })
}
