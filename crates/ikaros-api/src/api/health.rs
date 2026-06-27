// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::api) fn is_api_health_route(route: &str) -> bool {
    matches!(route, "/healthz" | "/health" | "/ready")
}

pub(in crate::api) fn ready_response(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ApiHttpResponse> {
    match ready_report(paths, workspace, agent_override) {
        Ok(body) => ApiHttpResponse::json(200, "OK", body),
        Err(error) => ApiHttpResponse::json(
            503,
            "Service Unavailable",
            json!({
                "status": "not_ready",
                "error": redact_secrets(&error.to_string()),
            }),
        ),
    }
}

pub(in crate::api) fn ready_report(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Value> {
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let model_config = agent.model_config(&config.model.default);
    Ok(json!({
        "status": "ready",
        "protocol": {
            "name": IKAROS_PROTOCOL_NAME,
            "version": IKAROS_PROTOCOL_VERSION,
        },
        "model": &model_config.model,
        "provider": &model_config.provider,
        "embedding_provider": &config.rag.embedding_provider,
        "embedding_model": api_embedding_model_id(&config),
        "image_generation_model": &model_config.model,
        "tts_model": &config.voice.tts.model,
        "asr_model": &config.voice.asr.model,
        "search_provider_configured": !config.providers.search.api_key.trim().is_empty()
            || !config.providers.search.base_url.trim().is_empty(),
        "workspace": workspace.display().to_string(),
    }))
}

pub(in crate::api) fn protocol_response() -> Result<ApiHttpResponse> {
    ApiHttpResponse::json(
        200,
        "OK",
        json!({
            "object": "ikaros.protocol",
            "name": IKAROS_PROTOCOL_NAME,
            "version": IKAROS_PROTOCOL_VERSION,
            "wire_envelope": {
                "protocol": IKAROS_PROTOCOL_NAME,
                "version": IKAROS_PROTOCOL_VERSION,
                "kind": "state_trace_entry",
            },
            "types": {
                "turn_state": "TurnState",
                "state_trace_entry": "StateTraceEntry",
                "turn_state_snapshot": "TurnStateSnapshot",
                "model_stream_event": "ModelStreamEvent",
                "model_request_diagnostic": "ModelRequestDiagnostic",
                "token_usage": "TokenUsage",
            },
            "routes": {
                "trace": "ikaros debug trace <session-id> [--turn-id <turn-id>]",
                "state_db": "ikaros debug state-db",
            },
            "stability": {
                "status": "pre_release",
                "compatibility": "versioned additive changes are preferred before 1.0",
            },
        }),
    )
}

pub(in crate::api) fn models_response(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ApiHttpResponse> {
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let model_config = agent.model_config(&config.model.default);
    let rows = [
        ApiModelRow {
            id: model_config.model.clone(),
            provider: model_config.provider.to_string(),
            capabilities: vec!["chat.completions", "responses", "images.generations"],
        },
        ApiModelRow {
            id: api_embedding_model_id(&config),
            provider: config.rag.embedding_provider.to_string(),
            capabilities: vec!["embeddings"],
        },
        ApiModelRow {
            id: config.voice.tts.model.clone(),
            provider: config.voice.tts.provider.to_string(),
            capabilities: vec!["audio.speech"],
        },
        ApiModelRow {
            id: config.voice.asr.model.clone(),
            provider: config.voice.asr.provider.to_string(),
            capabilities: vec!["audio.transcriptions"],
        },
    ];
    ApiHttpResponse::json(200, "OK", openai_models_response_body(rows))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::api) struct ApiModelRow {
    pub(in crate::api) id: String,
    pub(in crate::api) provider: String,
    pub(in crate::api) capabilities: Vec<&'static str>,
}

pub(in crate::api) fn api_embedding_model_id(config: &IkarosConfig) -> String {
    if config.rag.embedding_model.trim().is_empty() {
        config.rag.embedding_provider.to_string()
    } else {
        config.rag.embedding_model.clone()
    }
}

pub(in crate::api) fn openai_models_response_body(
    rows: impl IntoIterator<Item = ApiModelRow>,
) -> Value {
    let mut merged: BTreeMap<String, (String, Vec<&'static str>)> = BTreeMap::new();
    for row in rows {
        let id = row.id.trim();
        if id.is_empty() {
            continue;
        }
        let entry = merged
            .entry(id.to_owned())
            .or_insert_with(|| (row.provider.clone(), Vec::new()));
        for capability in row.capabilities {
            if !entry.1.contains(&capability) {
                entry.1.push(capability);
            }
        }
    }
    json!({
        "object": "list",
        "data": merged
            .into_iter()
            .map(|(id, (provider, capabilities))| {
                json!({
                    "id": id,
                    "object": "model",
                    "owned_by": "ikaros",
                    "ikaros": {
                        "provider": provider,
                        "capabilities": capabilities,
                    }
                })
            })
            .collect::<Vec<_>>(),
    })
}
