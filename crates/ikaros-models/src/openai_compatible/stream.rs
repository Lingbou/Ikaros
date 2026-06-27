// SPDX-License-Identifier: GPL-3.0-only

use super::{
    tools::{model_tool_calls, model_tool_calls_from_stream_accumulators},
    types::{ChatCompletionChunk, ChatCompletionResponse, OpenAiStreamToolCallAccumulator},
};
use crate::types::{
    ModelStream, ModelStreamEvent, ModelStreamEventSink, ModelToolCall, TokenUsage, chunk_text,
};
use ikaros_core::{IkarosError, Result, contains_secret_like, redact_secrets};

#[cfg(test)]
pub(crate) fn parse_stream_response(
    text: &str,
    provider: &str,
    fallback_model: &str,
) -> Result<ModelStream> {
    let mut sink = crate::types::NoopModelStreamEventSink;
    let mut accumulator = OpenAiStreamAccumulator::new(provider, fallback_model);
    accumulator.push_text(text, &mut sink)?;
    accumulator.finish(&mut sink)
}

pub(super) struct OpenAiStreamAccumulator {
    provider: String,
    fallback_model: String,
    raw_text: String,
    line_buffer: String,
    saw_sse_payload: bool,
    start_emitted: bool,
    chunks: Vec<String>,
    tool_call_accumulators: Vec<OpenAiStreamToolCallAccumulator>,
    content_redactor: StreamingSecretRedactor,
    reasoning_redactor: StreamingSecretRedactor,
    refusal_redactor: StreamingSecretRedactor,
    model: Option<String>,
    usage: TokenUsage,
    events: Vec<ModelStreamEvent>,
}

impl OpenAiStreamAccumulator {
    pub(super) fn new(provider: &str, fallback_model: &str) -> Self {
        Self {
            provider: provider.into(),
            fallback_model: fallback_model.into(),
            raw_text: String::new(),
            line_buffer: String::new(),
            saw_sse_payload: false,
            start_emitted: false,
            chunks: Vec::new(),
            tool_call_accumulators: Vec::new(),
            content_redactor: StreamingSecretRedactor::default(),
            reasoning_redactor: StreamingSecretRedactor::default(),
            refusal_redactor: StreamingSecretRedactor::default(),
            model: None,
            usage: TokenUsage::default(),
            events: Vec::new(),
        }
    }

    pub(super) fn push_text(
        &mut self,
        text: &str,
        sink: &mut dyn ModelStreamEventSink,
    ) -> Result<()> {
        self.raw_text.push_str(text);
        self.line_buffer.push_str(text);
        while let Some(newline) = self.line_buffer.find('\n') {
            let line = self.line_buffer.drain(..=newline).collect::<String>();
            self.process_line(line.trim_end_matches(['\r', '\n']), sink)?;
        }
        Ok(())
    }

    pub(super) fn finish(mut self, sink: &mut dyn ModelStreamEventSink) -> Result<ModelStream> {
        let trailing = std::mem::take(&mut self.line_buffer);
        if !trailing.trim().is_empty() {
            self.process_line(trailing.trim_end_matches('\r'), sink)?;
        }
        if !self.saw_sse_payload {
            if self.raw_text.trim_start().starts_with('{') {
                let stream = stream_from_chat_completion_json(
                    &self.raw_text,
                    &self.provider,
                    &self.fallback_model,
                )?;
                emit_stream_events_to_sink(&stream, sink)?;
                return Ok(stream);
            }
            return Err(IkarosError::Message(
                "model stream response did not contain content chunks".into(),
            ));
        }

        if let Some(content) = self.content_redactor.finish()
            && !content.is_empty()
        {
            self.emit_text_delta(content, sink)?;
        }
        if let Some(reasoning) = self.reasoning_redactor.finish()
            && !reasoning.is_empty()
        {
            self.emit_event(ModelStreamEvent::ReasoningDelta(reasoning), sink)?;
        }
        if let Some(refusal) = self.refusal_redactor.finish()
            && !refusal.is_empty()
        {
            self.emit_event(ModelStreamEvent::RefusalDelta(refusal), sink)?;
        }

        let tool_call_accumulators = std::mem::take(&mut self.tool_call_accumulators);
        for (index, accumulator) in tool_call_accumulators.iter().enumerate() {
            if stream_tool_call_has_payload(accumulator) {
                let id = stream_tool_call_id(index, accumulator);
                self.emit_event(
                    ModelStreamEvent::ToolCallStart {
                        id: id.clone(),
                        name: stream_tool_call_name(index, accumulator),
                    },
                    sink,
                )?;
                let redacted_arguments = redact_secrets(accumulator.arguments.trim());
                if !redacted_arguments.is_empty() {
                    self.emit_event(
                        ModelStreamEvent::ToolCallDelta {
                            id: id.clone(),
                            args_delta: redacted_arguments,
                        },
                        sink,
                    )?;
                }
                self.emit_event(ModelStreamEvent::ToolCallEnd { id }, sink)?;
            }
        }
        let tool_calls = model_tool_calls_from_stream_accumulators(tool_call_accumulators);
        let has_payload = !self.chunks.is_empty()
            || !tool_calls.is_empty()
            || self.events.iter().any(|event| {
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
        if self.usage.total_or_prompt_completion() > 0 {
            self.emit_event(ModelStreamEvent::Usage(self.usage.clone()), sink)?;
        }
        self.emit_event(ModelStreamEvent::Done, sink)?;

        Ok(ModelStream {
            provider: self.provider,
            model: self.model.unwrap_or(self.fallback_model),
            chunks: self.chunks,
            tool_calls,
            usage: self.usage,
            events: self.events,
            diagnostics: Vec::new(),
        })
    }

    fn process_line(&mut self, line: &str, sink: &mut dyn ModelStreamEventSink) -> Result<()> {
        let Some(payload) = line.trim().strip_prefix("data:") else {
            return Ok(());
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            return Ok(());
        }
        self.saw_sse_payload = true;
        let parsed: ChatCompletionChunk = serde_json::from_str(payload).map_err(|source| {
            IkarosError::Message(format!("failed to parse model stream chunk JSON: {source}"))
        })?;
        if self.model.is_none() {
            self.model = parsed.model.clone();
        }
        if let Some(next_usage) = parsed.usage {
            self.usage = next_usage;
        }
        self.ensure_start(sink)?;
        for choice in parsed.choices {
            if let Some(content) = choice.delta.content {
                for content in self.content_redactor.push(&content) {
                    if !content.is_empty() {
                        self.emit_text_delta(content, sink)?;
                    }
                }
            }
            if let Some(reasoning) = choice.delta.reasoning_content.or(choice.delta.reasoning) {
                for reasoning in self.reasoning_redactor.push(&reasoning) {
                    if !reasoning.is_empty() {
                        self.emit_event(ModelStreamEvent::ReasoningDelta(reasoning), sink)?;
                    }
                }
            }
            if let Some(refusal) = choice.delta.refusal {
                for refusal in self.refusal_redactor.push(&refusal) {
                    if !refusal.is_empty() {
                        self.emit_event(ModelStreamEvent::RefusalDelta(refusal), sink)?;
                    }
                }
            }
            accumulate_stream_tool_calls(&mut self.tool_call_accumulators, choice.delta.tool_calls);
        }
        Ok(())
    }

    fn ensure_start(&mut self, sink: &mut dyn ModelStreamEventSink) -> Result<()> {
        if self.start_emitted {
            return Ok(());
        }
        self.start_emitted = true;
        self.emit_event(
            ModelStreamEvent::Start {
                provider: self.provider.clone(),
                model: self
                    .model
                    .clone()
                    .unwrap_or_else(|| self.fallback_model.clone()),
            },
            sink,
        )
    }

    fn emit_text_delta(
        &mut self,
        content: String,
        sink: &mut dyn ModelStreamEventSink,
    ) -> Result<()> {
        self.chunks.push(content.clone());
        self.emit_event(ModelStreamEvent::TextDelta(content), sink)
    }

    fn emit_event(
        &mut self,
        event: ModelStreamEvent,
        sink: &mut dyn ModelStreamEventSink,
    ) -> Result<()> {
        sink.emit(event.clone())?;
        self.events.push(event);
        Ok(())
    }
}

fn stream_from_chat_completion_json(
    text: &str,
    provider: &str,
    fallback_model: &str,
) -> Result<ModelStream> {
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

fn emit_stream_events_to_sink(
    stream: &ModelStream,
    sink: &mut dyn ModelStreamEventSink,
) -> Result<()> {
    for event in stream.normalized_events() {
        sink.emit(event)?;
    }
    Ok(())
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

#[derive(Default)]
struct StreamingSecretRedactor {
    pending: String,
}

impl StreamingSecretRedactor {
    fn push(&mut self, fragment: &str) -> Vec<String> {
        if fragment.is_empty() {
            return Vec::new();
        }
        self.pending.push_str(fragment);
        let Some(split_at) = last_whitespace_boundary(&self.pending) else {
            return self.push_tail_safe_prefix();
        };
        let remainder = self.pending.split_off(split_at);
        let completed = std::mem::replace(&mut self.pending, remainder);
        let redacted = redact_secrets(&completed);
        if redacted.is_empty() {
            Vec::new()
        } else {
            vec![redacted]
        }
    }

    fn finish(&mut self) -> Option<String> {
        if self.pending.is_empty() {
            return None;
        }
        let pending = std::mem::take(&mut self.pending);
        Some(redact_secrets(&pending))
    }

    fn push_tail_safe_prefix(&mut self) -> Vec<String> {
        const TAIL_CHARS: usize = 24;
        if contains_secret_like(&self.pending) {
            return Vec::new();
        }
        let char_count = self.pending.chars().count();
        if char_count <= TAIL_CHARS {
            return Vec::new();
        }
        let split_chars = char_count - TAIL_CHARS;
        let split_at = self
            .pending
            .char_indices()
            .nth(split_chars)
            .map(|(index, _)| index)
            .unwrap_or(self.pending.len());
        let remainder = self.pending.split_off(split_at);
        let completed = std::mem::replace(&mut self.pending, remainder);
        let redacted = redact_secrets(&completed);
        if redacted.is_empty() {
            Vec::new()
        } else {
            vec![redacted]
        }
    }
}

fn last_whitespace_boundary(value: &str) -> Option<usize> {
    value
        .char_indices()
        .filter_map(|(index, ch)| ch.is_whitespace().then_some(index + ch.len_utf8()))
        .next_back()
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
