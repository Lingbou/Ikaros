// SPDX-License-Identifier: GPL-3.0-only

use super::{
    events::{emit_chat_event, emit_chat_failure_events},
    model::{
        cancellable_provider_generate, cancellable_provider_stream_with_events,
        model_messages_for_single_call_with_content_blocks, model_response_stream_events,
        redacted_chat_error,
    },
    result::ChatModelResult,
    setup::ChatTurnSetup,
};
use crate::AgentEventSink;
use ikaros_core::Result;
use ikaros_models::{
    ModelProvider, ModelRequest, ModelRequestDiagnostic, ModelRequestOptions, ModelResponse,
    ModelStreamEvent, ModelStreamEventSink,
};
use ikaros_session::{AgentEvent, AgentEventKind, AgentEventSource};

use crate::chat::types::ChatRunOptions;

pub(super) struct SingleCallInput<'a> {
    pub(super) input: &'a str,
    pub(super) provider: &'a dyn ModelProvider,
    pub(super) options: &'a ChatRunOptions,
    pub(super) request_options: ModelRequestOptions,
    pub(super) system_prompt_messages: &'a [String],
    pub(super) event_sink: &'a dyn AgentEventSink,
    pub(super) setup: &'a ChatTurnSetup,
}

pub(super) async fn run_single_call(
    input: SingleCallInput<'_>,
    events: &mut Vec<AgentEvent>,
) -> Result<ChatModelResult> {
    if input.options.stream {
        run_streaming_single_call(input, events).await
    } else {
        run_generate_single_call(input, events).await
    }
}

async fn run_streaming_single_call(
    input: SingleCallInput<'_>,
    events: &mut Vec<AgentEvent>,
) -> Result<ChatModelResult> {
    let request = ModelRequest {
        messages: model_messages_for_single_call_with_content_blocks(
            input.system_prompt_messages,
            input.input,
            &input.options.content_blocks,
        ),
        options: input.request_options,
        tools: Vec::new(),
    };
    let mut stream_sink = ChatModelStreamEventSink {
        events,
        event_sink: input.event_sink,
        setup: input.setup,
    };
    let stream = match cancellable_provider_stream_with_events(
        input.provider,
        request,
        input.options,
        &mut stream_sink,
    )
    .await
    {
        Ok(stream) => stream,
        Err(error) => {
            let error = redacted_chat_error(error);
            emit_chat_failure_events(
                events,
                input.event_sink,
                &input.setup.session_id,
                &input.setup.turn_id,
                "provider_stream",
                &error,
            )?;
            return Err(error);
        }
    };
    let response = ModelResponse {
        provider: stream.provider.clone(),
        model: stream.model.clone(),
        content: stream.content(),
        tool_calls: Vec::new(),
        usage: stream.usage.clone(),
        diagnostics: stream
            .diagnostics
            .into_iter()
            .map(ModelRequestDiagnostic::sanitized)
            .collect(),
    };
    Ok(ChatModelResult {
        response,
        streamed: true,
        stream_chunks: stream.chunks,
    })
}

struct ChatModelStreamEventSink<'a, 'b> {
    events: &'a mut Vec<AgentEvent>,
    event_sink: &'b dyn AgentEventSink,
    setup: &'b ChatTurnSetup,
}

impl ModelStreamEventSink for ChatModelStreamEventSink<'_, '_> {
    fn emit(&mut self, event: ModelStreamEvent) -> ikaros_core::Result<()> {
        emit_chat_event(
            self.events,
            self.event_sink,
            &self.setup.session_id,
            &self.setup.turn_id,
            AgentEventSource::Model,
            AgentEventKind::ModelStream(event),
            serde_json::Value::Null,
        )
    }
}

async fn run_generate_single_call(
    input: SingleCallInput<'_>,
    events: &mut Vec<AgentEvent>,
) -> Result<ChatModelResult> {
    let request = ModelRequest {
        messages: model_messages_for_single_call_with_content_blocks(
            input.system_prompt_messages,
            input.input,
            &input.options.content_blocks,
        ),
        options: input.request_options,
        tools: Vec::new(),
    };
    let response = match cancellable_provider_generate(input.provider, request, input.options).await
    {
        Ok(response) => response,
        Err(error) => {
            let error = redacted_chat_error(error);
            emit_chat_failure_events(
                events,
                input.event_sink,
                &input.setup.session_id,
                &input.setup.turn_id,
                "provider_generate",
                &error,
            )?;
            return Err(error);
        }
    };
    emit_model_stream_events(events, input.event_sink, input.setup, &response)?;
    Ok(ChatModelResult {
        response,
        streamed: false,
        stream_chunks: Vec::new(),
    })
}

fn emit_model_stream_events(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    setup: &ChatTurnSetup,
    response: &ModelResponse,
) -> Result<()> {
    for event in model_response_stream_events(response) {
        emit_chat_event(
            events,
            event_sink,
            &setup.session_id,
            &setup.turn_id,
            AgentEventSource::Model,
            AgentEventKind::ModelStream(event),
            serde_json::Value::Null,
        )?;
    }
    Ok(())
}
