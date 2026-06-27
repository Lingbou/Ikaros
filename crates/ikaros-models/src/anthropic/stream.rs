// SPDX-License-Identifier: GPL-3.0-only

use super::wire::{AnthropicStreamEvent, AnthropicStreamToolAccumulator, AnthropicUsage};
use crate::types::{ModelStream, ModelStreamEvent, ModelToolCall, TokenUsage, chunk_text};
use ikaros_core::{IkarosError, Result, redact_json, redact_secrets};
use std::collections::BTreeMap;

pub(crate) fn parse_stream_response(
    text: &str,
    provider: &str,
    fallback_model: &str,
) -> Result<ModelStream> {
    let mut model = None;
    let mut usage = AnthropicUsage::default();
    let mut content_text = String::new();
    let mut tools = BTreeMap::<usize, AnthropicStreamToolAccumulator>::new();

    for line in text.lines() {
        let Some(payload) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let event: AnthropicStreamEvent = serde_json::from_str(payload).map_err(|source| {
            IkarosError::Message(format!(
                "failed to parse Anthropic stream event JSON: {source}"
            ))
        })?;
        match event.kind.as_str() {
            "message_start" => {
                if let Some(message) = event.message {
                    if model.is_none() {
                        model = message.model;
                    }
                    if let Some(next_usage) = message.usage {
                        merge_anthropic_usage(&mut usage, next_usage);
                    }
                }
            }
            "content_block_start" => {
                if let (Some(index), Some(block)) = (event.index, event.content_block) {
                    match block.kind.as_str() {
                        "text" => {
                            if let Some(initial_text) = block.text {
                                content_text.push_str(&initial_text);
                            }
                        }
                        "tool_use" => {
                            tools.insert(
                                index,
                                AnthropicStreamToolAccumulator {
                                    id: block.id,
                                    name: block.name,
                                    arguments: String::new(),
                                },
                            );
                        }
                        _ => {}
                    }
                }
            }
            "content_block_delta" => {
                let Some(delta) = event.delta else {
                    continue;
                };
                match delta.kind.as_deref() {
                    Some("text_delta") => {
                        if let Some(delta_text) = delta.text {
                            content_text.push_str(&delta_text);
                        }
                    }
                    Some("input_json_delta") => {
                        if let (Some(index), Some(partial_json)) = (event.index, delta.partial_json)
                        {
                            tools
                                .entry(index)
                                .or_default()
                                .arguments
                                .push_str(&partial_json);
                        }
                    }
                    _ => {}
                }
            }
            "message_delta" => {
                if let Some(next_usage) = event.usage {
                    merge_anthropic_usage(&mut usage, next_usage);
                }
            }
            _ => {}
        }
    }

    let model = model.unwrap_or_else(|| fallback_model.into());
    let content = redact_secrets(&content_text);
    let chunks = if content.is_empty() {
        Vec::new()
    } else {
        chunk_text(&content, 96)
    };
    let tool_calls = tools
        .into_values()
        .filter_map(anthropic_stream_tool_call)
        .collect::<Vec<_>>();
    let usage = TokenUsage::from(usage);
    let has_payload = !chunks.is_empty() || !tool_calls.is_empty();
    if !has_payload {
        return Err(IkarosError::Message(
            "Anthropic stream response did not contain content chunks or tool calls".into(),
        ));
    }
    let events = model_stream_events_from_response(provider, &model, &chunks, &tool_calls, &usage);

    Ok(ModelStream {
        provider: provider.into(),
        model,
        chunks,
        tool_calls,
        usage,
        events,
        diagnostics: Vec::new(),
    })
}

fn anthropic_stream_tool_call(
    accumulator: AnthropicStreamToolAccumulator,
) -> Option<ModelToolCall> {
    let name = accumulator.name?;
    let raw_arguments = redact_secrets(&accumulator.arguments);
    let input = if accumulator.arguments.trim().is_empty() {
        serde_json::Value::Object(Default::default())
    } else {
        serde_json::from_str::<serde_json::Value>(&accumulator.arguments)
            .map(redact_json)
            .unwrap_or_else(|_| serde_json::json!({ "arguments": raw_arguments.clone() }))
    };
    Some(ModelToolCall {
        id: accumulator.id.map(|id| redact_secrets(&id)),
        name: redact_secrets(&name),
        raw_arguments: (!raw_arguments.trim().is_empty()).then_some(raw_arguments),
        input,
    })
}

fn merge_anthropic_usage(target: &mut AnthropicUsage, update: AnthropicUsage) {
    if update.input_tokens.is_some() {
        target.input_tokens = update.input_tokens;
    }
    if update.output_tokens.is_some() {
        target.output_tokens = update.output_tokens;
    }
    if update.cache_creation_input_tokens.is_some() {
        target.cache_creation_input_tokens = update.cache_creation_input_tokens;
    }
    if update.cache_read_input_tokens.is_some() {
        target.cache_read_input_tokens = update.cache_read_input_tokens;
    }
}

fn model_stream_events_from_response(
    provider: &str,
    model: &str,
    chunks: &[String],
    tool_calls: &[ModelToolCall],
    usage: &TokenUsage,
) -> Vec<ModelStreamEvent> {
    let mut events = vec![ModelStreamEvent::Start {
        provider: provider.into(),
        model: model.into(),
    }];
    events.extend(
        chunks
            .iter()
            .filter(|chunk| !chunk.is_empty())
            .cloned()
            .map(ModelStreamEvent::TextDelta),
    );
    for (index, call) in tool_calls.iter().enumerate() {
        let id = call
            .id
            .clone()
            .unwrap_or_else(|| format!("tool_call_{index}"));
        events.push(ModelStreamEvent::ToolCallStart {
            id: id.clone(),
            name: call.name.clone(),
        });
        if let Some(arguments) = &call.raw_arguments {
            events.push(ModelStreamEvent::ToolCallDelta {
                id: id.clone(),
                args_delta: arguments.clone(),
            });
        }
        events.push(ModelStreamEvent::ToolCallEnd { id });
    }
    if usage.total_or_prompt_completion() > 0 {
        events.push(ModelStreamEvent::Usage(usage.clone()));
    }
    events.push(ModelStreamEvent::Done);
    events
}

#[cfg(test)]
pub(crate) fn test_model_stream_events_from_response(
    provider: &str,
    model: &str,
    chunks: &[String],
    tool_calls: &[ModelToolCall],
    usage: &TokenUsage,
) -> Vec<ModelStreamEvent> {
    model_stream_events_from_response(provider, model, chunks, tool_calls, usage)
}
