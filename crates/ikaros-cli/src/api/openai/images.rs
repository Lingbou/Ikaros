// SPDX-License-Identifier: GPL-3.0-only

use super::super::*;

pub(in crate::api) fn image_generation_response(
    body: &[u8],
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ApiHttpResponse> {
    let mut request_body: Value = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                format!("invalid image generation JSON body: {error}"),
            ));
        }
    };
    let prompt = match image_generation_prompt(&request_body) {
        Ok(prompt) => prompt,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                error.to_string(),
            ));
        }
    };
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let model_config = agent.model_config(&config.model.default).clone();
    let model = image_generation_model(&mut request_body, &model_config.model)?;
    let evidence = ApiSessionEvidence::new(&agent, "/v1/images/generations", &model)?;
    evidence.append_entry(
        SessionEntryKind::Custom,
        Some("OpenAI-compatible image generation request".into()),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/images/generations",
            "model": &model,
            "prompt_preview": api_text_preview(&prompt),
            "prompt_chars": prompt.chars().count(),
            "size": request_body.get("size").and_then(Value::as_str),
            "n": request_body.get("n").and_then(Value::as_u64),
            "response_format": request_body.get("response_format").and_then(Value::as_str),
        }),
    )?;
    let provider = agent
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let request = match image_generation_http_request(&provider, &request_body) {
        Ok(request) => request,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                error.to_string(),
            ));
        }
    };
    let (session, _) = session_and_registry_for_instance(paths, &config, &agent)?;
    let client = EgressModelHttpClient::new(session.env.clone());
    let handle = tokio::runtime::Handle::current();
    let provider_response = tokio::task::block_in_place(|| handle.block_on(client.send(request)))
        .with_context(|| "image generation provider request failed")?;
    if !(200..300).contains(&provider_response.status) {
        evidence.emit(
            AgentEventSource::Runtime,
            AgentEventKind::TurnEnd,
            json!({
                "surface": "openai-compatible-api",
                "route": "/v1/images/generations",
                "status": "provider_error",
                "model": &model,
                "provider_status": provider_response.status,
                "provider_body_preview": api_text_preview(&provider_response.body),
            }),
        )?;
        let ids = evidence.ids();
        evidence.commit()?;
        return Ok(provider_image_error_response(
            provider_response.status,
            &provider_response.body,
        )
        .with_session(ids));
    }
    let mut response_body: Value = match serde_json::from_str(&provider_response.body) {
        Ok(body) => body,
        Err(error) => {
            evidence.emit(
                AgentEventSource::Runtime,
                AgentEventKind::TurnEnd,
                json!({
                    "surface": "openai-compatible-api",
                    "route": "/v1/images/generations",
                    "status": "invalid_provider_json",
                    "model": &model,
                    "error": redact_secrets(&error.to_string()),
                    "provider_body_preview": api_text_preview(&provider_response.body),
                }),
            )?;
            let ids = evidence.ids();
            evidence.commit()?;
            return Ok(ApiHttpResponse::json_error(
                502,
                "Bad Gateway",
                format!("image generation provider returned invalid JSON: {error}"),
            )
            .with_session(ids));
        }
    };
    let response_summary = image_generation_response_summary(&response_body);
    evidence.emit(
        AgentEventSource::Model,
        AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::Done),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/images/generations",
            "model": &model,
            "image_count": response_summary.image_count,
            "url_count": response_summary.url_count,
            "b64_json_count": response_summary.b64_json_count,
            "revised_prompt_count": response_summary.revised_prompt_count,
        }),
    )?;
    evidence.emit(
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/images/generations",
            "status": "completed",
            "model": &model,
            "image_count": response_summary.image_count,
            "url_count": response_summary.url_count,
            "b64_json_count": response_summary.b64_json_count,
            "revised_prompt_count": response_summary.revised_prompt_count,
            "items": response_summary.items,
        }),
    )?;
    let ids = evidence.ids();
    insert_api_session_metadata(&mut response_body, &ids);
    evidence.commit()?;
    Ok(ApiHttpResponse::json(200, "OK", response_body)?.with_session(ids))
}

pub(in crate::api) fn image_generation_prompt(body: &Value) -> Result<String> {
    let Some(prompt) = body
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|prompt| !prompt.is_empty())
    else {
        anyhow::bail!("image generation prompt must be a non-empty string");
    };
    Ok(prompt.to_owned())
}

pub(in crate::api) fn image_generation_model(
    body: &mut Value,
    default_model: &str,
) -> Result<String> {
    let Some(object) = body.as_object_mut() else {
        anyhow::bail!("image generation request body must be a JSON object");
    };
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_model.trim().to_owned());
    if model.is_empty() {
        anyhow::bail!("image generation model must not be empty");
    }
    object.insert("model".to_owned(), json!(&model));
    Ok(model)
}

pub(in crate::api) fn image_generation_http_request(
    provider: &RemoteProviderConfig,
    body: &Value,
) -> Result<ModelHttpRequest> {
    let base_url = provider.base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        anyhow::bail!("providers.model.base_url is required for image generation");
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
        url: format!("{base_url}/images/generations"),
        headers,
        body: serde_json::to_string(body)?,
    })
}

pub(in crate::api) fn provider_image_error_response(status: u16, body: &str) -> ApiHttpResponse {
    let api_status = if (400..500).contains(&status) {
        400
    } else {
        502
    };
    let reason = if api_status == 400 {
        "Bad Request"
    } else {
        "Bad Gateway"
    };
    ApiHttpResponse::json_error(
        api_status,
        reason,
        format!(
            "image generation provider returned HTTP {}: {}",
            status,
            api_text_preview(body)
        ),
    )
}

#[derive(Debug)]
pub(in crate::api) struct ApiImageGenerationSummary {
    pub(in crate::api) image_count: usize,
    pub(in crate::api) url_count: usize,
    pub(in crate::api) b64_json_count: usize,
    pub(in crate::api) revised_prompt_count: usize,
    pub(in crate::api) items: Vec<Value>,
}

pub(in crate::api) fn image_generation_response_summary(body: &Value) -> ApiImageGenerationSummary {
    let mut summary = ApiImageGenerationSummary {
        image_count: 0,
        url_count: 0,
        b64_json_count: 0,
        revised_prompt_count: 0,
        items: Vec::new(),
    };
    let Some(items) = body.get("data").and_then(Value::as_array) else {
        return summary;
    };
    summary.image_count = items.len();
    for (index, item) in items.iter().enumerate() {
        let url = item.get("url").and_then(Value::as_str);
        let b64_json = item.get("b64_json").and_then(Value::as_str);
        let revised_prompt = item.get("revised_prompt").and_then(Value::as_str);
        if url.is_some() {
            summary.url_count += 1;
        }
        if b64_json.is_some() {
            summary.b64_json_count += 1;
        }
        if revised_prompt.is_some() {
            summary.revised_prompt_count += 1;
        }
        summary.items.push(json!({
            "index": index,
            "url_preview": url.map(api_text_preview),
            "has_b64_json": b64_json.is_some(),
            "b64_json_bytes_estimate": b64_json.map(estimated_base64_decoded_len),
            "revised_prompt_preview": revised_prompt.map(api_text_preview),
        }));
    }
    summary
}

pub(in crate::api) fn estimated_base64_decoded_len(value: &str) -> usize {
    let trimmed = value.trim_end_matches('=');
    trimmed.len().saturating_mul(3) / 4
}

pub(in crate::api) fn insert_api_session_metadata(body: &mut Value, ids: &ApiSessionIds) {
    if let Some(object) = body.as_object_mut() {
        let ikaros = object.entry("ikaros").or_insert_with(|| json!({}));
        if let Some(ikaros) = ikaros.as_object_mut() {
            ikaros.insert("session_id".into(), json!(&ids.session_id));
            ikaros.insert("turn_id".into(), json!(&ids.turn_id));
        }
    }
}
