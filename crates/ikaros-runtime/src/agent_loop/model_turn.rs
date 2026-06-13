// SPDX-License-Identifier: GPL-3.0-only

use super::{
    dispatch::{
        dispatch_agent_loop_tool_call, model_message_for_tool_result,
        observe_agent_loop_tool_result, stop_reason_from_tool_result,
    },
    prompt::{
        agent_loop_tool_definitions, model_tool_definitions, render_agent_loop_system_prompt,
    },
    report::finish_agent_loop,
    stream::stream_chunks_for_final_content,
    tool_parse::{agent_loop_model_envelope_from_response, agent_loop_tool_call_diagnostic},
    types::{
        AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopFinish,
        AgentLoopInput, AgentLoopModelTurn, AgentLoopOptions, AgentLoopReport, AgentLoopStopReason,
        AgentSessionId, AgentTurnId,
    },
};
use ikaros_core::{Result, redact_secrets};
use ikaros_harness::{AuditEvent, ExecutionSession, GuardrailState, SkillRegistry};
use ikaros_models::{
    ModelMessage, ModelProvider, ModelRequest, ModelResponse, ModelStreamEvent, ModelToolCall,
    TokenUsage,
};
use serde_json::json;
use uuid::Uuid;

pub(super) async fn run_agent_loop_turn(
    input: AgentLoopInput,
    provider: &dyn ModelProvider,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    event_sink: &dyn AgentEventSink,
    options: AgentLoopOptions,
) -> Result<AgentLoopReport> {
    let tool_definitions = agent_loop_tool_definitions(registry);
    let session_id = input
        .task_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .unwrap_or("local")
        .to_owned();
    let turn_id = Uuid::new_v4().to_string();
    let mut events = Vec::new();
    emit_agent_event(
        &mut events,
        event_sink,
        &session_id,
        &turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::SessionStart,
        json!({
            "task_id": &input.task_id,
        }),
    )?;
    emit_agent_event(
        &mut events,
        event_sink,
        &session_id,
        &turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnStart,
        json!({
            "task_id": &input.task_id,
            "max_iterations": options.max_iterations,
            "stream": options.stream,
            "tool_count": tool_definitions.len(),
        }),
    )?;
    session.audit.append(AuditEvent::new(
        "agent_loop_start",
        None,
        "agent loop started",
        json!({
            "task_id": &input.task_id,
            "max_iterations": options.max_iterations,
            "stream": options.stream,
            "tool_count": tool_definitions.len(),
            "guardrails": &options.guardrails,
        }),
    )?)?;

    let mut messages = vec![
        ModelMessage::system(render_agent_loop_system_prompt(
            &input.system_prompt,
            &tool_definitions,
        )),
        ModelMessage::user(redact_secrets(&input.user_input)),
    ];
    emit_agent_event(
        &mut events,
        event_sink,
        &session_id,
        &turn_id,
        AgentEventSource::User,
        AgentEventKind::UserMessage,
        json!({
            "content": redact_secrets(&input.user_input),
        }),
    )?;
    let mut guardrails = GuardrailState::default();
    let mut tool_results = Vec::new();
    let mut tool_call_diagnostics = Vec::new();
    let max_iterations = options.max_iterations.max(1);
    let mut last_content = String::new();
    let mut last_provider = provider.name().to_string();
    let mut last_model = String::new();
    let mut total_usage = TokenUsage::default();
    let mut final_streamed = false;
    let mut final_stream_chunks = Vec::new();

    for iteration in 1..=max_iterations {
        let turn = match request_agent_loop_model_turn(
            provider,
            ModelRequest {
                messages: messages.clone(),
                max_tokens: options.max_tokens,
                temperature: options.temperature,
                tools: model_tool_definitions(&tool_definitions),
            },
            options.stream,
        )
        .await
        {
            Ok(turn) => turn,
            Err(error) => {
                emit_agent_event(
                    &mut events,
                    event_sink,
                    &session_id,
                    &turn_id,
                    AgentEventSource::Model,
                    AgentEventKind::Error,
                    json!({
                        "iteration": iteration,
                        "stop_reason": AgentLoopStopReason::ProviderError,
                        "message": redact_secrets(&error.to_string()),
                    }),
                )?;
                return Err(error);
            }
        };
        let response = turn.response;
        for event in &turn.stream_events {
            emit_agent_event(
                &mut events,
                event_sink,
                &session_id,
                &turn_id,
                AgentEventSource::Model,
                AgentEventKind::ModelStream(event.clone()),
                json!({
                    "iteration": iteration,
                }),
            )?;
        }
        last_provider = response.provider.clone();
        last_model = response.model.clone();
        total_usage = merge_token_usage(total_usage, &response.usage);
        let envelope = agent_loop_model_envelope_from_response(&response);
        let diagnostic = agent_loop_tool_call_diagnostic(iteration, &response, envelope.as_ref());
        let tool_call_count = envelope
            .as_ref()
            .map(|envelope| envelope.tool_calls.len())
            .unwrap_or_default();
        let final_answer = envelope
            .as_ref()
            .and_then(|envelope| envelope.final_answer.clone());
        session.audit.append(AuditEvent::new(
            "agent_loop_model_result",
            None,
            "agent loop model result",
            json!({
                "task_id": &input.task_id,
                "iteration": iteration,
                "provider": &response.provider,
                "model": &response.model,
                "streamed": turn.streamed,
                "stream_chunk_count": turn.stream_chunks.len(),
                "native_tool_call_count": response.tool_calls.len(),
                "tool_call_count": tool_call_count,
                "has_final_answer": final_answer.is_some(),
                "parse_strategy": diagnostic.strategy.as_str(),
                "repaired": diagnostic.repaired,
                "usage": &response.usage,
            }),
        )?)?;
        tool_call_diagnostics.push(diagnostic);

        if let Some(envelope) = envelope {
            last_content = envelope
                .final_answer
                .clone()
                .unwrap_or_else(|| response.content.clone());
            if envelope.tool_calls.is_empty() {
                if turn.streamed {
                    final_streamed = true;
                    final_stream_chunks =
                        stream_chunks_for_final_content(&turn.stream_chunks, &last_content);
                }
                return finish_agent_loop_turn(
                    session,
                    input.task_id,
                    &session_id,
                    &turn_id,
                    &mut events,
                    event_sink,
                    AgentLoopFinish {
                        stop_reason: AgentLoopStopReason::FinalAnswer,
                        final_content: last_content,
                        provider: last_provider,
                        model: last_model,
                        usage: total_usage,
                        streamed: final_streamed,
                        stream_chunks: final_stream_chunks,
                        iterations: iteration,
                        tool_call_diagnostics,
                        tool_results,
                        events: Vec::new(),
                    },
                );
            }
            messages.push(ModelMessage::assistant_with_tool_calls(
                redact_secrets(&response.content),
                response.tool_calls.clone(),
            ));
            for call in envelope.tool_calls {
                let tool_call_id = call.id.clone();
                let tool_call_name = call.name.clone();
                emit_agent_event(
                    &mut events,
                    event_sink,
                    &session_id,
                    &turn_id,
                    AgentEventSource::Tool,
                    AgentEventKind::ToolStart,
                    json!({
                        "iteration": iteration,
                        "id": &tool_call_id,
                        "name": &tool_call_name,
                        "input": &call.input,
                    }),
                )?;
                let tool_result =
                    dispatch_agent_loop_tool_call(session, registry, iteration, call).await;
                if tool_result.output.get("approval_id").is_some()
                    || tool_result
                        .output
                        .get("decision")
                        .and_then(serde_json::Value::as_str)
                        == Some("ask_user")
                {
                    emit_agent_event(
                        &mut events,
                        event_sink,
                        &session_id,
                        &turn_id,
                        AgentEventSource::Harness,
                        AgentEventKind::ApprovalRequested,
                        json!({
                            "iteration": iteration,
                            "tool": &tool_result.name,
                            "output": &tool_result.output,
                        }),
                    )?;
                }
                emit_agent_event(
                    &mut events,
                    event_sink,
                    &session_id,
                    &turn_id,
                    AgentEventSource::Tool,
                    AgentEventKind::ToolEnd,
                    json!({
                        "iteration": iteration,
                        "name": &tool_result.name,
                        "ok": tool_result.ok,
                        "summary": &tool_result.summary,
                        "output": &tool_result.output,
                    }),
                )?;
                let stop = observe_agent_loop_tool_result(
                    session,
                    input.task_id.as_deref(),
                    &mut guardrails,
                    &options.guardrails,
                    &tool_result,
                )?;
                messages.push(model_message_for_tool_result(
                    tool_call_id,
                    tool_call_name,
                    &tool_result,
                ));
                let stop_reason = stop.or_else(|| stop_reason_from_tool_result(&tool_result));
                tool_results.push(tool_result);
                if let Some(stop_reason) = stop_reason {
                    return finish_agent_loop_turn(
                        session,
                        input.task_id,
                        &session_id,
                        &turn_id,
                        &mut events,
                        event_sink,
                        AgentLoopFinish {
                            stop_reason,
                            final_content: last_content,
                            provider: last_provider,
                            model: last_model,
                            usage: total_usage,
                            streamed: final_streamed,
                            stream_chunks: final_stream_chunks,
                            iterations: iteration,
                            tool_call_diagnostics,
                            tool_results,
                            events: Vec::new(),
                        },
                    );
                }
            }
            continue;
        }

        last_content = response.content;
        if turn.streamed {
            final_streamed = true;
            final_stream_chunks =
                stream_chunks_for_final_content(&turn.stream_chunks, &last_content);
        }
        return finish_agent_loop_turn(
            session,
            input.task_id,
            &session_id,
            &turn_id,
            &mut events,
            event_sink,
            AgentLoopFinish {
                stop_reason: AgentLoopStopReason::FinalAnswer,
                final_content: last_content,
                provider: last_provider,
                model: last_model,
                usage: total_usage,
                streamed: final_streamed,
                stream_chunks: final_stream_chunks,
                iterations: iteration,
                tool_call_diagnostics,
                tool_results,
                events: Vec::new(),
            },
        );
    }

    finish_agent_loop_turn(
        session,
        input.task_id,
        &session_id,
        &turn_id,
        &mut events,
        event_sink,
        AgentLoopFinish {
            stop_reason: AgentLoopStopReason::IterationBudget,
            final_content: last_content,
            provider: last_provider,
            model: last_model,
            usage: total_usage,
            streamed: final_streamed,
            stream_chunks: final_stream_chunks,
            iterations: max_iterations,
            tool_call_diagnostics,
            tool_results,
            events: Vec::new(),
        },
    )
}

async fn request_agent_loop_model_turn(
    provider: &dyn ModelProvider,
    request: ModelRequest,
    stream: bool,
) -> Result<AgentLoopModelTurn> {
    if stream {
        let stream = provider.stream(request).await?;
        let stream_events = stream.normalized_events();
        let response = ModelResponse {
            provider: stream.provider.clone(),
            model: stream.model.clone(),
            content: stream.content(),
            tool_calls: stream.tool_calls,
            usage: stream.usage.clone(),
        };
        return Ok(AgentLoopModelTurn {
            response,
            streamed: true,
            stream_chunks: stream.chunks,
            stream_events,
        });
    }

    let response = provider.generate(request).await?;
    let stream_events = model_response_stream_events(&response);
    Ok(AgentLoopModelTurn {
        response,
        streamed: false,
        stream_chunks: Vec::new(),
        stream_events,
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

fn emit_agent_event(
    events: &mut Vec<AgentEvent>,
    sink: &dyn AgentEventSink,
    session_id: &AgentSessionId,
    turn_id: &AgentTurnId,
    source: AgentEventSource,
    kind: AgentEventKind,
    payload: serde_json::Value,
) -> Result<()> {
    let parent_event_id = events.last().map(|event| event.event_id.clone());
    let event = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        parent_event_id,
        source,
        kind,
        payload,
    );
    sink.emit(&event)?;
    events.push(event);
    Ok(())
}

fn finish_agent_loop_turn(
    session: &ExecutionSession,
    task_id: Option<String>,
    session_id: &AgentSessionId,
    turn_id: &AgentTurnId,
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    mut finish: AgentLoopFinish,
) -> Result<AgentLoopReport> {
    emit_agent_event(
        events,
        event_sink,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "stop_reason": &finish.stop_reason,
            "iterations": finish.iterations,
            "tool_result_count": finish.tool_results.len(),
        }),
    )?;
    finish.events = std::mem::take(events);
    finish_agent_loop(session, task_id, finish)
}

fn merge_token_usage(mut total: TokenUsage, usage: &TokenUsage) -> TokenUsage {
    total.prompt_tokens = merge_token_count(total.prompt_tokens, usage.prompt_tokens);
    total.completion_tokens = merge_token_count(total.completion_tokens, usage.completion_tokens);
    total.total_tokens = merge_token_count(total.total_tokens, usage.total_tokens);
    total
}

fn merge_token_count(left: Option<u32>, right: Option<u32>) -> Option<u32> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}
