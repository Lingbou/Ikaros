// SPDX-License-Identifier: GPL-3.0-only

use super::super::*;
use super::*;

pub(in crate::api) fn audio_speech_response(
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
                format!("invalid audio speech JSON body: {error}"),
            ));
        }
    };
    let input = match audio_speech_input(&request_body) {
        Ok(input) => input,
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
    let model = match audio_speech_prepare_body(
        &mut request_body,
        &config.voice.tts.model,
        config.voice.tts.voice.as_deref(),
    ) {
        Ok(model) => model,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                error.to_string(),
            ));
        }
    };
    let format = request_body
        .get("response_format")
        .and_then(Value::as_str)
        .unwrap_or("mp3")
        .to_owned();
    let evidence = ApiSessionEvidence::new(&agent, "/v1/audio/speech", &model)?;
    evidence.append_entry(
        SessionEntryKind::Custom,
        Some("OpenAI-compatible audio speech request".into()),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/audio/speech",
            "model": &model,
            "input_preview": api_text_preview(&input),
            "input_chars": input.chars().count(),
            "voice": request_body.get("voice").and_then(Value::as_str),
            "response_format": &format,
        }),
    )?;
    let provider_response = send_api_json_provider_request(
        paths,
        &config,
        &agent,
        &config.providers.tts,
        "/audio/speech",
        &request_body,
    )?;
    if !(200..300).contains(&provider_response.status) {
        evidence.emit(
            AgentEventSource::Runtime,
            AgentEventKind::TurnEnd,
            json!({
                "surface": "openai-compatible-api",
                "route": "/v1/audio/speech",
                "status": "provider_error",
                "model": &model,
                "provider_status": provider_response.status,
                "provider_body_preview": api_text_preview(&provider_response.body),
            }),
        )?;
        let ids = evidence.ids();
        evidence.commit()?;
        return Ok(provider_audio_error_response(
            provider_response.status,
            &provider_response.body,
        )
        .with_session(ids));
    }
    let audio = provider_response
        .body_bytes
        .unwrap_or_else(|| provider_response.body.into_bytes());
    evidence.emit(
        AgentEventSource::Model,
        AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::Done),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/audio/speech",
            "model": &model,
            "audio_bytes": audio.len(),
            "response_format": &format,
        }),
    )?;
    evidence.emit(
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/audio/speech",
            "status": "completed",
            "model": &model,
            "audio_bytes": audio.len(),
            "response_format": &format,
        }),
    )?;
    let ids = evidence.ids();
    evidence.commit()?;
    Ok(ApiHttpResponse::binary(200, "OK", audio_content_type(&format), audio).with_session(ids))
}

pub(in crate::api) fn audio_transcription_response(
    body: &[u8],
    headers: &ApiHeaders,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ApiHttpResponse> {
    let content_type = headers.content_type.as_deref().unwrap_or_default();
    let form = match parse_api_multipart_form(body, content_type) {
        Ok(form) => form,
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
    let model = form
        .fields
        .get("model")
        .map(String::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(config.voice.asr.model.trim())
        .to_owned();
    if model.is_empty() {
        return Ok(ApiHttpResponse::json_error(
            400,
            "Bad Request",
            "audio transcription model must not be empty",
        ));
    }
    let response_format = form
        .fields
        .get("response_format")
        .map(String::as_str)
        .unwrap_or("json")
        .to_owned();
    let evidence = ApiSessionEvidence::new(&agent, "/v1/audio/transcriptions", &model)?;
    evidence.append_entry(
        SessionEntryKind::Custom,
        Some("OpenAI-compatible audio transcription request".into()),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/audio/transcriptions",
            "model": &model,
            "file_name": &form.file_name,
            "file_bytes": form.file.len(),
            "language": form.fields.get("language"),
            "response_format": &response_format,
        }),
    )?;
    let (multipart_type, multipart_body) = api_asr_multipart_body(&model, &form);
    let provider_response = send_api_bytes_provider_request(
        paths,
        &config,
        &agent,
        &config.providers.asr,
        "/audio/transcriptions",
        multipart_type,
        multipart_body,
    )?;
    if !(200..300).contains(&provider_response.status) {
        evidence.emit(
            AgentEventSource::Runtime,
            AgentEventKind::TurnEnd,
            json!({
                "surface": "openai-compatible-api",
                "route": "/v1/audio/transcriptions",
                "status": "provider_error",
                "model": &model,
                "provider_status": provider_response.status,
                "provider_body_preview": api_text_preview(&provider_response.body),
            }),
        )?;
        let ids = evidence.ids();
        evidence.commit()?;
        return Ok(provider_audio_error_response(
            provider_response.status,
            &provider_response.body,
        )
        .with_session(ids));
    }
    let response_bytes = provider_response
        .body_bytes
        .unwrap_or_else(|| provider_response.body.into_bytes());
    let response_text = String::from_utf8_lossy(&response_bytes).into_owned();
    evidence.emit(
        AgentEventSource::Model,
        AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::Done),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/audio/transcriptions",
            "model": &model,
            "response_format": &response_format,
            "response_bytes": response_bytes.len(),
        }),
    )?;
    evidence.emit(
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/audio/transcriptions",
            "status": "completed",
            "model": &model,
            "response_format": &response_format,
            "response_bytes": response_bytes.len(),
        }),
    )?;
    let ids = evidence.ids();
    evidence.commit()?;
    if transcription_response_is_json(&response_format) {
        let mut response_body: Value = match serde_json::from_str(&response_text) {
            Ok(body) => body,
            Err(error) => {
                return Ok(ApiHttpResponse::json_error(
                    502,
                    "Bad Gateway",
                    format!(
                        "audio transcription provider returned non-JSON for a JSON response_format: {error}"
                    ),
                )
                .with_session(ids));
            }
        };
        insert_api_session_metadata(&mut response_body, &ids);
        return Ok(ApiHttpResponse::json(200, "OK", response_body)?.with_session(ids));
    }
    Ok(ApiHttpResponse::binary(
        200,
        "OK",
        transcription_content_type(&response_format),
        response_bytes,
    )
    .with_session(ids))
}

pub(in crate::api) fn audio_speech_input(body: &Value) -> Result<String> {
    let Some(input) = body
        .get("input")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|input| !input.is_empty())
    else {
        anyhow::bail!("audio speech input must be a non-empty string");
    };
    Ok(input.to_owned())
}

pub(in crate::api) fn audio_speech_prepare_body(
    body: &mut Value,
    default_model: &str,
    default_voice: Option<&str>,
) -> Result<String> {
    let Some(object) = body.as_object_mut() else {
        anyhow::bail!("audio speech request body must be a JSON object");
    };
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_model.trim().to_owned());
    if model.is_empty() {
        anyhow::bail!("audio speech model must not be empty");
    }
    object.insert("model".into(), json!(&model));
    if object
        .get("voice")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|voice| !voice.is_empty())
        .is_none()
    {
        object.insert("voice".into(), json!(default_voice.unwrap_or("alloy")));
    }
    if object
        .get("response_format")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|format| !format.is_empty())
        .is_none()
    {
        object.insert("response_format".into(), json!("mp3"));
    }
    Ok(model)
}

pub(in crate::api) fn send_api_json_provider_request(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent: &AgentInstance,
    provider: &RemoteProviderConfig,
    path: &str,
    body: &Value,
) -> Result<ikaros_harness::NetworkEgressResponse> {
    let (session, _) = session_and_registry_for_instance(paths, config, agent)?;
    let mut headers = BTreeMap::new();
    headers.insert("content-type".into(), "application/json".into());
    if !provider.api_key.trim().is_empty() {
        headers.insert(
            "authorization".into(),
            format!("Bearer {}", provider.api_key.trim()),
        );
    }
    let request = provider_network_request(
        provider,
        path,
        headers,
        Some(serde_json::to_string(body)?),
        None,
    )?;
    let handle = tokio::runtime::Handle::current();
    let response =
        tokio::task::block_in_place(|| handle.block_on(session.env.send_network_request(request)))?;
    Ok(response)
}

pub(in crate::api) fn send_api_bytes_provider_request(
    paths: &IkarosPaths,
    config: &IkarosConfig,
    agent: &AgentInstance,
    provider: &RemoteProviderConfig,
    path: &str,
    content_type: String,
    body: Vec<u8>,
) -> Result<ikaros_harness::NetworkEgressResponse> {
    let (session, _) = session_and_registry_for_instance(paths, config, agent)?;
    let mut headers = BTreeMap::new();
    headers.insert("content-type".into(), content_type);
    if !provider.api_key.trim().is_empty() {
        headers.insert(
            "authorization".into(),
            format!("Bearer {}", provider.api_key.trim()),
        );
    }
    let request = provider_network_request(provider, path, headers, None, Some(body))?;
    let handle = tokio::runtime::Handle::current();
    let response =
        tokio::task::block_in_place(|| handle.block_on(session.env.send_network_request(request)))?;
    Ok(response)
}

pub(in crate::api) fn provider_network_request(
    provider: &RemoteProviderConfig,
    path: &str,
    headers: BTreeMap<String, String>,
    body: Option<String>,
    body_bytes: Option<Vec<u8>>,
) -> Result<NetworkEgressRequest> {
    let base_url = provider.base_url.trim().trim_end_matches('/');
    if base_url.is_empty() {
        anyhow::bail!("provider base_url is required for API provider proxy routes");
    }
    Ok(NetworkEgressRequest {
        method: "POST".into(),
        url: format!("{base_url}{path}"),
        headers,
        body,
        body_bytes,
    })
}

pub(in crate::api) fn provider_audio_error_response(status: u16, body: &str) -> ApiHttpResponse {
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
            "audio provider returned HTTP {}: {}",
            status,
            api_text_preview(body)
        ),
    )
}

pub(in crate::api) fn audio_content_type(format: &str) -> &'static str {
    match format.trim().to_ascii_lowercase().as_str() {
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "opus" => "audio/ogg",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        _ => "audio/mpeg",
    }
}

pub(in crate::api) fn transcription_response_is_json(format: &str) -> bool {
    matches!(
        format.trim().to_ascii_lowercase().as_str(),
        "" | "json" | "verbose_json"
    )
}

pub(in crate::api) fn transcription_content_type(format: &str) -> &'static str {
    match format.trim().to_ascii_lowercase().as_str() {
        "srt" => "application/x-subrip; charset=utf-8",
        "vtt" => "text/vtt; charset=utf-8",
        _ => "text/plain; charset=utf-8",
    }
}
