// SPDX-License-Identifier: GPL-3.0-only

use super::super::*;
use super::*;

pub(in crate::api) fn embedding_response(
    body: &[u8],
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ApiHttpResponse> {
    let request: ApiEmbeddingRequest = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                format!("invalid embedding JSON body: {error}"),
            ));
        }
    };
    let encoding = match request.embedding_encoding() {
        Ok(encoding) => encoding,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                error.to_string(),
            ));
        }
    };
    let inputs = match request.inputs() {
        Ok(inputs) => inputs,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                error.to_string(),
            ));
        }
    };
    if inputs.is_empty() {
        return Ok(ApiHttpResponse::json_error(
            400,
            "Bad Request",
            "embedding input must not be empty",
        ));
    }
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let mut rag_config = config.rag.clone();
    if let Some(model) = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        rag_config.embedding_model = model.to_owned();
    }
    let response_model = if rag_config.embedding_model.trim().is_empty() {
        rag_config.embedding_provider.to_string()
    } else {
        rag_config.embedding_model.clone()
    };
    let evidence = ApiSessionEvidence::new(&agent, "/v1/embeddings", &response_model)?;
    evidence.append_entry(
        SessionEntryKind::Custom,
        Some("OpenAI-compatible embedding request".into()),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/embeddings",
            "embedding_provider": rag_config.embedding_provider,
            "model": response_model,
            "encoding_format": encoding.as_str(),
            "input_count": inputs.len(),
            "input_chars": inputs.iter().map(|input| input.chars().count()).sum::<usize>(),
        }),
    )?;
    let (session, _) = session_and_registry_for_instance(paths, &config, &agent)?;
    let vectors = with_execution_env_embedding_provider(
        &rag_config,
        &config.providers.embedding,
        session.env.clone(),
        |provider| {
            inputs
                .iter()
                .map(|input| provider.embed(input))
                .collect::<ikaros_core::Result<Vec<_>>>()
        },
    )?;
    let prompt_tokens = inputs
        .iter()
        .map(|input| estimate_embedding_tokens(input))
        .sum::<u32>();
    evidence.emit(
        AgentEventSource::Model,
        AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::Done),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/embeddings",
            "embedding_provider": rag_config.embedding_provider,
            "model": response_model,
            "encoding_format": encoding.as_str(),
            "vector_count": vectors.len(),
            "prompt_tokens": prompt_tokens,
        }),
    )?;
    evidence.emit(
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/embeddings",
            "embedding_provider": rag_config.embedding_provider,
            "model": response_model,
            "encoding_format": encoding.as_str(),
            "vector_count": vectors.len(),
            "prompt_tokens": prompt_tokens,
        }),
    )?;
    let ids = evidence.ids();
    evidence.commit()?;
    Ok(ApiHttpResponse::json(
        200,
        "OK",
        openai_embedding_response_body(
            response_model,
            vectors,
            prompt_tokens,
            encoding,
            Some(&ids),
        ),
    )?
    .with_session(ids))
}
