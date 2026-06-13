// SPDX-License-Identifier: GPL-3.0-only

use super::{
    tools::{model_tool_calls, model_tool_calls_from_stream_accumulators},
    types::{ChatCompletionChunk, ChatCompletionResponse, OpenAiStreamToolCallAccumulator},
};
use crate::types::{ModelStream, TokenUsage, chunk_text};
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
        return Ok(ModelStream {
            provider: provider.into(),
            model: parsed.model.unwrap_or_else(|| fallback_model.into()),
            chunks,
            tool_calls: model_tool_calls(
                parsed
                    .choices
                    .first()
                    .map(|choice| choice.message.tool_calls.as_slice())
                    .unwrap_or_default(),
            ),
            usage: parsed.usage.unwrap_or_default(),
            events: Vec::new(),
        });
    }

    let mut chunks = Vec::new();
    let mut tool_call_accumulators = Vec::<OpenAiStreamToolCallAccumulator>::new();
    let mut model = None;
    let mut usage = TokenUsage::default();
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
                    chunks.push(content);
                }
            }
            accumulate_stream_tool_calls(&mut tool_call_accumulators, choice.delta.tool_calls);
        }
    }

    let tool_calls = model_tool_calls_from_stream_accumulators(tool_call_accumulators);
    if chunks.is_empty() && tool_calls.is_empty() {
        return Err(IkarosError::Message(
            "model stream response did not contain content chunks".into(),
        ));
    }

    Ok(ModelStream {
        provider: provider.into(),
        model: model.unwrap_or_else(|| fallback_model.into()),
        chunks,
        tool_calls,
        usage,
        events: Vec::new(),
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
