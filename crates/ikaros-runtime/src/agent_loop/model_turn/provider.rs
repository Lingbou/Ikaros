// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::types::AgentLoopModelTurn;
use ikaros_core::{Result, redact_secrets};
use ikaros_models::{
    ModelProvider, ModelRequest, ModelRequestDiagnostic, ModelResponse, ModelStreamEvent,
    ModelStreamEventSink, ModelToolCall,
};

pub(super) async fn request_agent_loop_model_turn(
    provider: &dyn ModelProvider,
    request: ModelRequest,
    stream: bool,
    stream_event_sink: Option<&mut dyn ModelStreamEventSink>,
) -> Result<AgentLoopModelTurn> {
    if stream {
        let stream_events_already_emitted = stream_event_sink.is_some();
        let stream = if let Some(sink) = stream_event_sink {
            provider.stream_with_events(request, sink).await?
        } else {
            provider.stream(request).await?
        };
        let stream_events = stream.normalized_events();
        let response = ModelResponse {
            provider: stream.provider.clone(),
            model: stream.model.clone(),
            content: stream.content(),
            tool_calls: stream.tool_calls,
            usage: stream.usage.clone(),
            diagnostics: stream
                .diagnostics
                .into_iter()
                .map(ModelRequestDiagnostic::sanitized)
                .collect(),
        };
        return Ok(AgentLoopModelTurn {
            response,
            streamed: true,
            stream_chunks: stream.chunks,
            stream_events,
            stream_events_already_emitted,
        });
    }

    let response = provider.generate(request).await?;
    let stream_events = model_response_stream_events(&response);
    Ok(AgentLoopModelTurn {
        response,
        streamed: false,
        stream_chunks: Vec::new(),
        stream_events,
        stream_events_already_emitted: false,
    })
}

fn model_response_stream_events(response: &ModelResponse) -> Vec<ModelStreamEvent> {
    let mut events = vec![ModelStreamEvent::Start {
        provider: response.provider.clone(),
        model: response.model.clone(),
    }];
    if !response.content.is_empty() {
        events.push(ModelStreamEvent::TextDelta(redact_secrets(
            &response.content,
        )));
    }
    events.extend(model_tool_call_stream_events(&response.tool_calls));
    if response.usage.total_or_prompt_completion() > 0 {
        events.push(ModelStreamEvent::Usage(response.usage.clone()));
    }
    events.push(ModelStreamEvent::Done);
    events
}

fn model_tool_call_stream_events(calls: &[ModelToolCall]) -> Vec<ModelStreamEvent> {
    let mut events = Vec::new();
    for call in calls {
        let id = call.id.clone().unwrap_or_else(|| call.name.clone());
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
    events
}
