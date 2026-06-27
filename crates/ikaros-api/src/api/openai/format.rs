// SPDX-License-Identifier: GPL-3.0-only

use super::super::*;
use super::*;

pub(in crate::api) fn openai_tool_calls_json(calls: &[ModelToolCall]) -> Vec<Value> {
    calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            json!({
                "id": call.id.clone().unwrap_or_else(|| format!("call_{index}")),
                "type": "function",
                "function": {
                    "name": &call.name,
                    "arguments": call.raw_arguments.clone().unwrap_or_else(|| call.input.to_string()),
                }
            })
        })
        .collect()
}

pub(in crate::api) fn openai_assistant_message_json(
    content: &str,
    calls: &[ModelToolCall],
) -> Value {
    let mut message = json!({
        "role": "assistant",
        "content": if content.is_empty() && !calls.is_empty() {
            Value::Null
        } else {
            Value::String(content.to_owned())
        },
    });
    if !calls.is_empty() {
        message["tool_calls"] = Value::Array(openai_tool_calls_json(calls));
    }
    message
}

pub(in crate::api) fn openai_finish_reason(calls: &[ModelToolCall]) -> &'static str {
    if calls.is_empty() {
        "stop"
    } else {
        "tool_calls"
    }
}

pub(in crate::api) fn emit_api_model_tool_call_events(
    evidence: &ApiSessionEvidence,
    calls: &[ModelToolCall],
) -> Result<()> {
    for (index, call) in calls.iter().enumerate() {
        let call_id = call.id.clone().unwrap_or_else(|| format!("call_{index}"));
        evidence.emit(
            AgentEventSource::Model,
            AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::ToolCallStart {
                id: call_id.clone(),
                name: call.name.clone(),
            }),
            json!({
                "surface": "openai-compatible-api",
                "tool_call_id": call_id,
                "tool_name": &call.name,
            }),
        )?;
        let call_id = call.id.clone().unwrap_or_else(|| format!("call_{index}"));
        evidence.emit(
            AgentEventSource::Model,
            AgentEventKind::ModelStream(ikaros_models::ModelStreamEvent::ToolCallEnd {
                id: call_id.clone(),
            }),
            json!({
                "surface": "openai-compatible-api",
                "tool_call_id": call_id,
                "tool_name": &call.name,
            }),
        )?;
    }
    Ok(())
}

pub(in crate::api) fn openai_usage_json(usage: &TokenUsage) -> Value {
    json!({
        "prompt_tokens": usage.prompt_tokens.unwrap_or_default(),
        "completion_tokens": usage.completion_tokens.unwrap_or_default(),
        "total_tokens": usage.total_or_prompt_completion(),
        "prompt_tokens_details": {
            "cached_tokens": usage.cache_read_tokens.unwrap_or_default(),
        },
        "completion_tokens_details": {},
    })
}

pub(in crate::api) fn openai_embedding_response_body(
    model: String,
    vectors: Vec<Vec<f32>>,
    prompt_tokens: u32,
    encoding: ApiEmbeddingEncoding,
    session: Option<&ApiSessionIds>,
) -> Value {
    let data = vectors
        .into_iter()
        .enumerate()
        .map(|(index, embedding)| {
            json!({
                "object": "embedding",
                "index": index,
                "embedding": openai_embedding_value(embedding, encoding),
            })
        })
        .collect::<Vec<_>>();
    let mut body = json!({
        "object": "list",
        "data": data,
        "model": model,
        "usage": {
            "prompt_tokens": prompt_tokens,
            "total_tokens": prompt_tokens,
        },
    });
    if let Some(session) = session {
        body["ikaros"] = json!({
            "session_id": session.session_id,
            "turn_id": session.turn_id,
        });
    }
    body
}

pub(in crate::api) struct ApiResponsesBody {
    pub(in crate::api) content: String,
    pub(in crate::api) model: String,
    pub(in crate::api) provider: String,
    pub(in crate::api) tool_calls: Vec<ModelToolCall>,
    pub(in crate::api) usage: TokenUsage,
    pub(in crate::api) diagnostics: Vec<ikaros_models::ModelRequestDiagnostic>,
    pub(in crate::api) created: i64,
}

pub(in crate::api) fn responses_response_body(
    response: ApiResponsesBody,
    session: Option<&ApiSessionIds>,
) -> Value {
    let ApiResponsesBody {
        content,
        model,
        provider,
        tool_calls,
        usage,
        diagnostics,
        created,
    } = response;
    let id = format!("resp-{created}");
    let mut output = vec![json!({
        "id": format!("msg-{created}"),
        "type": "message",
        "status": "completed",
        "role": "assistant",
        "content": [{
            "type": "output_text",
            "text": content.clone(),
            "annotations": [],
        }],
    })];
    output.extend(responses_function_call_output_items(&tool_calls));
    let mut body = json!({
        "id": id,
        "object": "response",
        "created_at": created,
        "status": "completed",
        "model": model,
        "output": output,
        "output_text": content,
        "usage": responses_usage_json(&usage),
        "error": Value::Null,
        "incomplete_details": Value::Null,
        "metadata": {},
        "ikaros": {
            "provider": provider,
            "diagnostics": diagnostics,
        }
    });
    if let Some(session) = session {
        body["ikaros"]["session_id"] = json!(&session.session_id);
        body["ikaros"]["turn_id"] = json!(&session.turn_id);
    }
    body
}

pub(in crate::api) fn responses_function_call_output_items(calls: &[ModelToolCall]) -> Vec<Value> {
    calls
        .iter()
        .enumerate()
        .map(|(index, call)| {
            json!({
                "id": call.id.clone().unwrap_or_else(|| format!("fc_{index}")),
                "type": "function_call",
                "status": "completed",
                "call_id": call.id.clone().unwrap_or_else(|| format!("call_{index}")),
                "name": &call.name,
                "arguments": call.raw_arguments.clone().unwrap_or_else(|| call.input.to_string()),
            })
        })
        .collect()
}

pub(in crate::api) fn responses_usage_json(usage: &TokenUsage) -> Value {
    json!({
        "input_tokens": usage.prompt_tokens.unwrap_or_default(),
        "output_tokens": usage.completion_tokens.unwrap_or_default(),
        "total_tokens": usage.total_or_prompt_completion(),
        "input_tokens_details": {
            "cached_tokens": usage.cache_read_tokens.unwrap_or_default(),
        },
        "output_tokens_details": {},
    })
}

pub(in crate::api) fn openai_embedding_value(
    embedding: Vec<f32>,
    encoding: ApiEmbeddingEncoding,
) -> Value {
    match encoding {
        ApiEmbeddingEncoding::Float => json!(embedding),
        ApiEmbeddingEncoding::Base64 => {
            let mut bytes = Vec::with_capacity(embedding.len() * std::mem::size_of::<f32>());
            for value in embedding {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
            Value::String(base64_encode(&bytes))
        }
    }
}

pub(in crate::api) fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        encoded.push(TABLE[(b0 >> 2) as usize] as char);
        encoded.push(TABLE[(((b0 & 0b0000_0011) << 4) | (b1 >> 4)) as usize] as char);
        if chunk.len() > 1 {
            encoded.push(TABLE[(((b1 & 0b0000_1111) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            encoded.push('=');
        }
        if chunk.len() > 2 {
            encoded.push(TABLE[(b2 & 0b0011_1111) as usize] as char);
        } else {
            encoded.push('=');
        }
    }
    encoded
}

pub(in crate::api) fn estimate_embedding_tokens(input: &str) -> u32 {
    input
        .split_whitespace()
        .map(|part| part.len().div_ceil(4).max(1) as u32)
        .sum::<u32>()
        .max(1)
}

pub(in crate::api) fn openai_stream_body(
    stream: &ModelStream,
    created: i64,
    session: Option<&ApiSessionIds>,
) -> Result<String> {
    let id = format!("chatcmpl-{created}");
    let mut body = String::new();
    push_sse_json(
        &mut body,
        json!({
            "id": &id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": &stream.model,
            "choices": [{
                "index": 0,
                "delta": {"role": "assistant"},
                "finish_reason": null
            }]
        }),
    )?;
    for chunk in &stream.chunks {
        if chunk.is_empty() {
            continue;
        }
        push_sse_json(
            &mut body,
            json!({
                "id": &id,
                "object": "chat.completion.chunk",
                "created": created,
                "model": &stream.model,
                "choices": [{
                    "index": 0,
                    "delta": {"content": chunk},
                    "finish_reason": null
                }]
            }),
        )?;
    }
    let mut final_delta = json!({});
    if !stream.tool_calls.is_empty() {
        final_delta["tool_calls"] = Value::Array(openai_tool_calls_json(&stream.tool_calls));
    }
    let mut final_chunk = json!({
            "id": &id,
            "object": "chat.completion.chunk",
            "created": created,
            "model": &stream.model,
            "choices": [{
                "index": 0,
                "delta": final_delta,
                "finish_reason": openai_finish_reason(&stream.tool_calls)
            }],
            "usage": openai_usage_json(&stream.usage),
            "ikaros": {
                "provider": &stream.provider,
                "diagnostics": &stream.diagnostics,
            }
    });
    if let Some(session) = session {
        final_chunk["ikaros"]["session_id"] = json!(&session.session_id);
        final_chunk["ikaros"]["turn_id"] = json!(&session.turn_id);
    }
    push_sse_json(&mut body, final_chunk)?;
    body.push_str("data: [DONE]\n\n");
    Ok(body)
}

pub(in crate::api) fn responses_stream_body(
    stream: &ModelStream,
    created: i64,
    session: Option<&ApiSessionIds>,
) -> Result<String> {
    let id = format!("resp-{created}");
    let mut body = String::new();
    push_sse_event_json(
        &mut body,
        "response.created",
        json!({
            "type": "response.created",
            "response": {
                "id": &id,
                "object": "response",
                "created_at": created,
                "status": "in_progress",
                "model": &stream.model,
                "output": [],
                "output_text": "",
            }
        }),
    )?;
    let mut full_text = String::new();
    for chunk in &stream.chunks {
        if chunk.is_empty() {
            continue;
        }
        full_text.push_str(chunk);
        push_sse_event_json(
            &mut body,
            "response.output_text.delta",
            json!({
                "type": "response.output_text.delta",
                "response_id": &id,
                "output_index": 0,
                "content_index": 0,
                "delta": chunk,
            }),
        )?;
    }
    push_sse_event_json(
        &mut body,
        "response.output_text.done",
        json!({
            "type": "response.output_text.done",
            "response_id": &id,
            "output_index": 0,
            "content_index": 0,
            "text": &full_text,
        }),
    )?;
    push_sse_event_json(
        &mut body,
        "response.completed",
        responses_response_body(
            ApiResponsesBody {
                content: full_text,
                model: stream.model.clone(),
                provider: stream.provider.clone(),
                tool_calls: stream.tool_calls.clone(),
                usage: stream.usage.clone(),
                diagnostics: stream.diagnostics.clone(),
                created,
            },
            session,
        ),
    )?;
    body.push_str("data: [DONE]\n\n");
    Ok(body)
}

pub(in crate::api) fn push_sse_json(body: &mut String, value: Value) -> Result<()> {
    body.push_str("data: ");
    body.push_str(&serde_json::to_string(&value)?);
    body.push_str("\n\n");
    Ok(())
}

pub(in crate::api) fn push_sse_event_json(
    body: &mut String,
    event: &str,
    value: Value,
) -> Result<()> {
    body.push_str("event: ");
    body.push_str(event);
    body.push('\n');
    push_sse_json(body, value)
}
