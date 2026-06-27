// SPDX-License-Identifier: GPL-3.0-only

use super::super::*;
use super::*;

pub(in crate::api) fn chat_completion_response(
    body: &[u8],
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ApiHttpResponse> {
    let request: ApiChatCompletionRequest = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                format!("invalid chat completion JSON body: {error}"),
            ));
        }
    };
    let stream_response = request.stream.unwrap_or(false);
    if request.messages.is_empty() {
        return Ok(ApiHttpResponse::json_error(
            400,
            "Bad Request",
            "messages must not be empty",
        ));
    }
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let mut model_config = agent.model_config(&config.model.default).clone();
    if let Some(model) = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        model_config.model = model.to_owned();
    }
    let evidence = ApiSessionEvidence::new(&agent, "/v1/chat/completions", &model_config.model)?;
    evidence.append_entry(
        SessionEntryKind::Custom,
        Some("OpenAI-compatible chat completion request".into()),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/chat/completions",
            "stream": stream_response,
            "model": model_config.model,
            "message_count": request.messages.len(),
            "tool_count": request.tools.len(),
        }),
    )?;
    let model_provider = agent
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let (session, _) = session_and_registry_for_instance(paths, &config, &agent)?;
    let provider = governed_provider_from_config_with_http_client(
        &model_config,
        &model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(session.env.clone()))),
    )?;
    let options = match api_request_options(&model_config, &request) {
        Ok(options) => options,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                error.to_string(),
            ));
        }
    };
    let tools = request
        .tools
        .into_iter()
        .map(api_tool_definition_to_model_tool)
        .collect::<Result<Vec<_>>>();
    let tools = match tools {
        Ok(tools) => tools,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                error.to_string(),
            ));
        }
    };
    let messages = request
        .messages
        .into_iter()
        .map(api_message_to_model_message)
        .collect::<Result<Vec<_>>>();
    let messages = match messages {
        Ok(messages) => messages,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                error.to_string(),
            ));
        }
    };
    for message in messages.iter().filter(|message| message.role == "user") {
        let preview = api_text_preview(&message.content);
        evidence.append_entry(
            SessionEntryKind::UserMessage,
            Some(preview.clone()),
            json!({
                "surface": "openai-compatible-api",
                "route": "/v1/chat/completions",
                "role": message.role,
                "content_preview": preview,
                "content_chars": message.content.chars().count(),
                "content_block_count": message.content_blocks.len(),
            }),
        )?;
        evidence.emit(
            AgentEventSource::User,
            AgentEventKind::UserMessage,
            json!({
                "surface": "openai-compatible-api",
                "route": "/v1/chat/completions",
                "content_preview": api_text_preview(&message.content),
                "content_chars": message.content.chars().count(),
                "content_block_count": message.content_blocks.len(),
            }),
        )?;
    }
    let model_request = ModelRequest {
        messages,
        options,
        tools,
    };
    let handle = tokio::runtime::Handle::current();
    let created = OffsetDateTime::now_utc().unix_timestamp().max(0);
    if stream_response {
        let stream =
            tokio::task::block_in_place(|| handle.block_on(provider.stream(model_request)))
                .with_context(|| "model provider stream request failed")?;
        for chunk in stream.chunks.iter().filter(|chunk| !chunk.is_empty()) {
            evidence.emit(
                AgentEventSource::Model,
                AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::TextDelta(
                    api_text_preview(chunk),
                )),
                json!({
                    "surface": "openai-compatible-api",
                    "route": "/v1/chat/completions",
                    "chunk_chars": chunk.chars().count(),
                }),
            )?;
        }
        emit_api_model_tool_call_events(&evidence, &stream.tool_calls)?;
        evidence.emit(
            AgentEventSource::Runtime,
            AgentEventKind::TurnEnd,
            json!({
                "surface": "openai-compatible-api",
                "route": "/v1/chat/completions",
                "stream": true,
                "model": &stream.model,
                "provider": &stream.provider,
                "usage": openai_usage_json(&stream.usage),
                "tool_call_count": stream.tool_calls.len(),
                "diagnostic_count": stream.diagnostics.len(),
            }),
        )?;
        let ids = evidence.ids();
        evidence.commit()?;
        return Ok(ApiHttpResponse::event_stream(openai_stream_body(
            &stream,
            created,
            Some(&ids),
        )?)
        .with_session(ids));
    }
    let response =
        tokio::task::block_in_place(|| handle.block_on(provider.generate(model_request)))
            .with_context(|| "model provider request failed")?;
    let assistant_preview = api_text_preview(&response.content);
    evidence.append_entry(
        SessionEntryKind::AssistantMessage,
        Some(assistant_preview.clone()),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/chat/completions",
            "model": &response.model,
            "provider": &response.provider,
            "content_preview": &assistant_preview,
            "content_chars": response.content.chars().count(),
            "tool_call_count": response.tool_calls.len(),
            "usage": openai_usage_json(&response.usage),
        }),
    )?;
    evidence.emit(
        AgentEventSource::Model,
        AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::TextDelta(
            api_text_preview(&response.content),
        )),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/chat/completions",
            "content_chars": response.content.chars().count(),
        }),
    )?;
    emit_api_model_tool_call_events(&evidence, &response.tool_calls)?;
    evidence.emit(
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/chat/completions",
            "stream": false,
            "model": &response.model,
            "provider": &response.provider,
            "usage": openai_usage_json(&response.usage),
            "tool_call_count": response.tool_calls.len(),
            "diagnostic_count": response.diagnostics.len(),
        }),
    )?;
    let ids = evidence.ids();
    evidence.commit()?;
    let response_body = json!({
        "id": format!("chatcmpl-{}", created),
        "object": "chat.completion",
        "created": created,
        "model": &response.model,
        "choices": [{
            "index": 0,
            "message": openai_assistant_message_json(&response.content, &response.tool_calls),
            "finish_reason": openai_finish_reason(&response.tool_calls)
        }],
        "usage": openai_usage_json(&response.usage),
        "ikaros": {
            "provider": &response.provider,
            "diagnostics": &response.diagnostics,
            "session_id": ids.session_id,
            "turn_id": ids.turn_id,
        }
    });
    Ok(ApiHttpResponse::json(200, "OK", response_body)?.with_session(ids))
}

pub(in crate::api) fn responses_response(
    body: &[u8],
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ApiHttpResponse> {
    let request: ApiResponseCreateRequest = match serde_json::from_slice(body) {
        Ok(request) => request,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                format!("invalid response JSON body: {error}"),
            ));
        }
    };
    let config = IkarosConfig::load_shape_checked(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let mut model_config = agent.model_config(&config.model.default).clone();
    if let Some(model) = request
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
    {
        model_config.model = model.to_owned();
    }
    let stream_response = request.stream.unwrap_or(false);
    let evidence = ApiSessionEvidence::new(&agent, "/v1/responses", &model_config.model)?;
    evidence.append_entry(
        SessionEntryKind::Custom,
        Some("OpenAI-compatible response request".into()),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/responses",
            "stream": stream_response,
            "model": model_config.model,
        }),
    )?;
    let (session, _) = session_and_registry_for_instance(paths, &config, &agent)?;
    let model_provider = agent
        .effective_model_provider_config(&config.model.default, &config.providers.model)
        .clone();
    let provider = governed_provider_from_config_with_http_client(
        &model_config,
        &model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(session.env.clone()))),
    )?;
    let mut options = model_request_options_from_config(&model_config)?;
    if let Some(max_output_tokens) = request.max_output_tokens {
        options.max_tokens = Some(max_output_tokens);
    }
    if let Some(temperature) = request.temperature {
        options.temperature = Some(temperature);
    }
    if let Some(top_p) = request.top_p {
        options.top_p = Some(top_p);
    }
    let mut messages = match api_responses_input_to_model_messages(request.input) {
        Ok(messages) => messages,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                error.to_string(),
            ));
        }
    };
    if let Some(instructions) = request
        .instructions
        .map(|instructions| instructions.trim().to_owned())
        .filter(|instructions| !instructions.is_empty())
    {
        messages.insert(0, ModelMessage::system(instructions));
    }
    if messages.is_empty() {
        return Ok(ApiHttpResponse::json_error(
            400,
            "Bad Request",
            "responses input must not be empty",
        ));
    }
    for message in messages.iter().filter(|message| message.role == "user") {
        let preview = api_text_preview(&message.content);
        evidence.append_entry(
            SessionEntryKind::UserMessage,
            Some(preview.clone()),
            json!({
                "surface": "openai-compatible-api",
                "route": "/v1/responses",
                "role": message.role,
                "content_preview": preview,
                "content_chars": message.content.chars().count(),
                "content_block_count": message.content_blocks.len(),
            }),
        )?;
        evidence.emit(
            AgentEventSource::User,
            AgentEventKind::UserMessage,
            json!({
                "surface": "openai-compatible-api",
                "route": "/v1/responses",
                "content_preview": api_text_preview(&message.content),
                "content_chars": message.content.chars().count(),
                "content_block_count": message.content_blocks.len(),
            }),
        )?;
    }
    let tools = match request
        .tools
        .into_iter()
        .map(api_response_tool_to_model_tool)
        .collect::<Result<Vec<_>>>()
    {
        Ok(tools) => tools,
        Err(error) => {
            return Ok(ApiHttpResponse::json_error(
                400,
                "Bad Request",
                error.to_string(),
            ));
        }
    };
    let model_request = ModelRequest {
        messages,
        options,
        tools,
    };
    let handle = tokio::runtime::Handle::current();
    let created = OffsetDateTime::now_utc().unix_timestamp().max(0);
    if stream_response {
        let stream =
            tokio::task::block_in_place(|| handle.block_on(provider.stream(model_request)))
                .with_context(|| "model provider stream request failed")?;
        for chunk in stream.chunks.iter().filter(|chunk| !chunk.is_empty()) {
            evidence.emit(
                AgentEventSource::Model,
                AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::TextDelta(
                    api_text_preview(chunk),
                )),
                json!({
                    "surface": "openai-compatible-api",
                    "route": "/v1/responses",
                    "chunk_chars": chunk.chars().count(),
                }),
            )?;
        }
        emit_api_model_tool_call_events(&evidence, &stream.tool_calls)?;
        evidence.emit(
            AgentEventSource::Runtime,
            AgentEventKind::TurnEnd,
            json!({
                "surface": "openai-compatible-api",
                "route": "/v1/responses",
                "stream": true,
                "model": &stream.model,
                "provider": &stream.provider,
                "usage": openai_usage_json(&stream.usage),
                "tool_call_count": stream.tool_calls.len(),
                "diagnostic_count": stream.diagnostics.len(),
            }),
        )?;
        let ids = evidence.ids();
        evidence.commit()?;
        return Ok(ApiHttpResponse::event_stream(responses_stream_body(
            &stream,
            created,
            Some(&ids),
        )?)
        .with_session(ids));
    }
    let response =
        tokio::task::block_in_place(|| handle.block_on(provider.generate(model_request)))
            .with_context(|| "model provider request failed")?;
    let assistant_preview = api_text_preview(&response.content);
    evidence.append_entry(
        SessionEntryKind::AssistantMessage,
        Some(assistant_preview.clone()),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/responses",
            "model": &response.model,
            "provider": &response.provider,
            "content_preview": &assistant_preview,
            "content_chars": response.content.chars().count(),
            "tool_call_count": response.tool_calls.len(),
            "usage": openai_usage_json(&response.usage),
        }),
    )?;
    evidence.emit(
        AgentEventSource::Model,
        AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::TextDelta(
            api_text_preview(&response.content),
        )),
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/responses",
            "content_chars": response.content.chars().count(),
        }),
    )?;
    emit_api_model_tool_call_events(&evidence, &response.tool_calls)?;
    evidence.emit(
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "surface": "openai-compatible-api",
            "route": "/v1/responses",
            "stream": false,
            "model": &response.model,
            "provider": &response.provider,
            "usage": openai_usage_json(&response.usage),
            "tool_call_count": response.tool_calls.len(),
            "diagnostic_count": response.diagnostics.len(),
        }),
    )?;
    let ids = evidence.ids();
    evidence.commit()?;
    Ok(ApiHttpResponse::json(
        200,
        "OK",
        responses_response_body(
            ApiResponsesBody {
                content: response.content,
                model: response.model,
                provider: response.provider,
                tool_calls: response.tool_calls,
                usage: response.usage,
                diagnostics: response.diagnostics,
                created,
            },
            Some(&ids),
        ),
    )?
    .with_session(ids))
}
