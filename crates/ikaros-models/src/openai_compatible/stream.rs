// SPDX-License-Identifier: GPL-3.0-only

use super::{
    tools::{model_tool_calls, model_tool_calls_from_stream_accumulators},
    types::{ChatCompletionChunk, ChatCompletionResponse, OpenAiStreamToolCallAccumulator},
};
use crate::types::{ModelStream, ModelStreamEvent, ModelToolCall, TokenUsage, chunk_text};
use ikaros_core::{IkarosError, Result, redact_secrets};

pub(crate) fn parse_stream_response(
    text: &str,
    provider: &str,
    fallback_model: &str,
) -> Result<ModelStream> {
    if text.trim_start().starts_with('{') {
        let parsed: ChatCompletionResponse = serde_json::from_str(text).map_err(|source| {
            IkarosError::Message(format!("failed to parse model response JSON: {source}"))
        })?;
        let content = parsed
            .choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
            .map(redact_secrets)
            .unwrap_or_default();
        let chunks = if content.is_empty() {
            Vec::new()
        } else {
            chunk_text(&content, 96)
        };
        let model = parsed.model.unwrap_or_else(|| fallback_model.into());
        let tool_calls = model_tool_calls(
            parsed
                .choices
                .first()
                .map(|choice| choice.message.tool_calls.as_slice())
                .unwrap_or_default(),
        );
        let usage = parsed.usage.unwrap_or_default();
        let events = stream_events_from_response(provider, &model, &chunks, &tool_calls, &usage);
        return Ok(ModelStream {
            provider: provider.into(),
            model,
            chunks,
            tool_calls,
            usage,
            events,
        });
    }

    let mut chunks = Vec::new();
    let mut tool_call_accumulators = Vec::<OpenAiStreamToolCallAccumulator>::new();
    let mut model = None;
    let mut usage = TokenUsage::default();
    let mut events = Vec::<ModelStreamEvent>::new();
    for line in text.lines() {
        let Some(payload) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        let parsed: ChatCompletionChunk = serde_json::from_str(payload).map_err(|source| {
            IkarosError::Message(format!("failed to parse model stream chunk JSON: {source}"))
        })?;
        if model.is_none() {
            model = parsed.model;
        }
        if let Some(next_usage) = parsed.usage {
            usage = next_usage;
        }
        for choice in parsed.choices {
            if let Some(content) = choice.delta.content {
                let content = redact_secrets(&content);
                if !content.is_empty() {
                    events.push(ModelStreamEvent::TextDelta(content.clone()));
                    chunks.push(content);
                }
            }
            if let Some(reasoning) = choice
                .delta
                .reasoning_content
                .or(choice.delta.reasoning)
                .map(|reasoning| redact_secrets(&reasoning))
            {
                if !reasoning.is_empty() {
                    events.push(ModelStreamEvent::ReasoningDelta(reasoning));
                }
            }
            if let Some(refusal) = choice.delta.refusal.map(|refusal| redact_secrets(&refusal)) {
                if !refusal.is_empty() {
                    events.push(ModelStreamEvent::RefusalDelta(refusal));
                }
            }
            accumulate_stream_tool_calls(&mut tool_call_accumulators, choice.delta.tool_calls);
        }
    }

    for (index, accumulator) in tool_call_accumulators.iter().enumerate() {
        if stream_tool_call_has_payload(accumulator) {
            let id = stream_tool_call_id(index, accumulator);
            events.push(ModelStreamEvent::ToolCallStart {
                id: id.clone(),
                name: stream_tool_call_name(index, accumulator),
            });
            let redacted_arguments = redact_secrets(accumulator.arguments.trim());
            if !redacted_arguments.is_empty() {
                events.push(ModelStreamEvent::ToolCallDelta {
                    id: id.clone(),
                    args_delta: redacted_arguments,
                });
            }
            events.push(ModelStreamEvent::ToolCallEnd { id });
        }
    }
    let tool_calls = model_tool_calls_from_stream_accumulators(tool_call_accumulators);
    let has_payload = !chunks.is_empty()
        || !tool_calls.is_empty()
        || events.iter().any(|event| {
            matches!(
                event,
                ModelStreamEvent::ReasoningDelta(_) | ModelStreamEvent::RefusalDelta(_)
            )
        });
    if !has_payload {
        return Err(IkarosError::Message(
            "model stream response did not contain content chunks".into(),
        ));
    }
    let model = model.unwrap_or_else(|| fallback_model.into());
    events.insert(
        0,
        ModelStreamEvent::Start {
            provider: provider.into(),
            model: model.clone(),
        },
    );
    if usage.total_or_prompt_completion() > 0 {
        events.push(ModelStreamEvent::Usage(usage.clone()));
    }
    events.push(ModelStreamEvent::Done);

    Ok(ModelStream {
        provider: provider.into(),
        model,
        chunks,
        tool_calls,
        usage,
        events,
    })
}

fn accumulate_stream_tool_calls(
    accumulators: &mut Vec<OpenAiStreamToolCallAccumulator>,
    deltas: Vec<super::types::OpenAiStreamToolCallDelta>,
) {
    for (position, delta) in deltas.into_iter().enumerate() {
        let index = delta.index.unwrap_or(position);
        if accumulators.len() <= index {
            accumulators.resize_with(index + 1, OpenAiStreamToolCallAccumulator::default);
        }
        let accumulator = &mut accumulators[index];
        if let Some(id) = delta.id {
            accumulator.id = Some(redact_secrets(&id));
        }
        if let Some(function) = delta.function {
            if let Some(name) = function.name {
                accumulator.name.push_str(&name);
            }
            if let Some(arguments) = function.arguments {
                accumulator.arguments.push_str(&arguments);
            }
        }
    }
}

fn stream_tool_call_has_payload(accumulator: &OpenAiStreamToolCallAccumulator) -> bool {
    accumulator.id.is_some()
        || !accumulator.name.trim().is_empty()
        || !accumulator.arguments.trim().is_empty()
}

fn stream_tool_call_id(index: usize, accumulator: &OpenAiStreamToolCallAccumulator) -> String {
    accumulator
        .id
        .clone()
        .unwrap_or_else(|| format!("tool_call_{index}"))
}

fn stream_tool_call_name(index: usize, accumulator: &OpenAiStreamToolCallAccumulator) -> String {
    let name = accumulator.name.trim();
    if name.is_empty() {
        format!("tool_call_{index}")
    } else {
        redact_secrets(name)
    }
}

fn stream_events_from_response(
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
