// SPDX-License-Identifier: GPL-3.0-only

use super::{
    dispatch::{
        dispatch_agent_loop_tool_call, model_message_for_tool_result,
        observe_agent_loop_tool_result, stop_reason_from_tool_result, tool_result_cancelled,
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
        AgentLoopToolCall, AgentLoopToolDefinition, AgentLoopToolResult, AgentSessionId,
        AgentTurnId,
    },
};
use ikaros_core::{IkarosError, Result, redact_json, redact_secrets};
use ikaros_harness::{
    AuditEvent, CancellationToken, ExecutionSession, GuardrailState, SkillRegistry,
    ToolExecutionMode,
};
use ikaros_models::{
    ModelMessage, ModelProvider, ModelRequest, ModelResponse, ModelStreamEvent, ModelToolCall,
    TokenUsage,
};
use ikaros_session::{
    AgentEventId, ApprovalRecord as SessionApprovalRecord, ApprovalStatus as SessionApprovalStatus,
    SessionId, TurnId,
};
use serde_json::json;
use time::OffsetDateTime;

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
        .session_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .map(SessionId::from)
        .unwrap_or_default();
    let turn_id = input
        .turn_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .map(TurnId::from)
        .unwrap_or_default();
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

    if options.cancellation.is_cancelled() {
        return finish_agent_loop_turn(
            session,
            input.task_id,
            &session_id,
            &turn_id,
            &mut events,
            event_sink,
            AgentLoopFinish {
                stop_reason: AgentLoopStopReason::Cancelled,
                final_content: last_content,
                provider: last_provider,
                model: last_model,
                usage: total_usage,
                streamed: final_streamed,
                stream_chunks: final_stream_chunks,
                iterations: 0,
                tool_call_diagnostics,
                tool_results,
                events: Vec::new(),
            },
        );
    }

    for iteration in 1..=max_iterations {
        if options.cancellation.is_cancelled() {
            return finish_agent_loop_turn(
                session,
                input.task_id,
                &session_id,
                &turn_id,
                &mut events,
                event_sink,
                AgentLoopFinish {
                    stop_reason: AgentLoopStopReason::Cancelled,
                    final_content: last_content,
                    provider: last_provider,
                    model: last_model,
                    usage: total_usage,
                    streamed: final_streamed,
                    stream_chunks: final_stream_chunks,
                    iterations: iteration.saturating_sub(1),
                    tool_call_diagnostics,
                    tool_results,
                    events: Vec::new(),
                },
            );
        }
        let turn = match request_agent_loop_model_turn(
            provider,
            ModelRequest {
                messages: messages.clone(),
                options: options.request_options.clone(),
                tools: model_tool_definitions(&tool_definitions),
            },
            options.stream,
        )
        .await
        {
            Ok(turn) => turn,
            Err(error) => {
                let error = IkarosError::Message(redact_secrets(&error.to_string()));
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
                emit_agent_event(
                    &mut events,
                    event_sink,
                    &session_id,
                    &turn_id,
                    AgentEventSource::Runtime,
                    AgentEventKind::TurnEnd,
                    json!({
                        "stop_reason": AgentLoopStopReason::ProviderError,
                        "iterations": iteration,
                        "tool_result_count": tool_results.len(),
                        "status": "failed",
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
                "diagnostics": &response.diagnostics,
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
            let scheduled_calls =
                schedule_agent_loop_tool_calls(envelope.tool_calls, &tool_definitions);
            if options.cancellation.is_cancelled() {
                tool_results.extend(emit_cancelled_tool_call_events(
                    ToolBatchDispatchContext {
                        session,
                        registry,
                        cancellation: &options.cancellation,
                        session_id: &session_id,
                        turn_id: &turn_id,
                        events: &mut events,
                        event_sink,
                    },
                    iteration,
                    &scheduled_calls,
                )?);
                return finish_agent_loop_turn(
                    session,
                    input.task_id,
                    &session_id,
                    &turn_id,
                    &mut events,
                    event_sink,
                    AgentLoopFinish {
                        stop_reason: AgentLoopStopReason::Cancelled,
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
            let mut scheduled_index = 0;
            while scheduled_index < scheduled_calls.len() {
                let batch_end = scheduled_tool_batch_end(&scheduled_calls, scheduled_index);
                let batch = scheduled_calls[scheduled_index..batch_end].to_vec();
                let dispatched = dispatch_scheduled_tool_batch(
                    ToolBatchDispatchContext {
                        session,
                        registry,
                        cancellation: &options.cancellation,
                        session_id: &session_id,
                        turn_id: &turn_id,
                        events: &mut events,
                        event_sink,
                    },
                    iteration,
                    batch,
                )
                .await?;
                for dispatched_tool in dispatched {
                    let tool_call_id = dispatched_tool.tool_call_id;
                    let tool_call_name = dispatched_tool.tool_call_name;
                    let started_event_id = dispatched_tool.started_event_id;
                    let tool_result = dispatched_tool.result;
                    if tool_result_waiting_for_approval(&tool_result) {
                        emit_agent_event(
                            &mut events,
                            event_sink,
                            &session_id,
                            &turn_id,
                            AgentEventSource::Harness,
                            AgentEventKind::ApprovalRequested,
                            json!({
                                "iteration": iteration,
                                "tool_call_id": &tool_call_id,
                                "tool_event_id": started_event_id.as_str(),
                                "tool": &tool_result.name,
                                "output": &tool_result.output,
                            }),
                        )?;
                        emit_session_approval_record(
                            event_sink,
                            session,
                            &session_id,
                            &turn_id,
                            &tool_result.output,
                        )?;
                    }
                    emit_agent_event(
                        &mut events,
                        event_sink,
                        &session_id,
                        &turn_id,
                        AgentEventSource::Tool,
                        AgentEventKind::ToolCallOutputDelta,
                        json!({
                            "iteration": iteration,
                            "tool_call_id": &tool_call_id,
                            "tool_event_id": started_event_id.as_str(),
                            "name": &tool_result.name,
                            "summary": &tool_result.summary,
                            "output": &tool_result.output,
                        }),
                    )?;
                    emit_agent_event(
                        &mut events,
                        event_sink,
                        &session_id,
                        &turn_id,
                        AgentEventSource::Tool,
                        tool_lifecycle_end_kind(&tool_result),
                        json!({
                            "iteration": iteration,
                            "tool_call_id": &tool_call_id,
                            "tool_event_id": started_event_id.as_str(),
                            "name": &tool_result.name,
                            "ok": tool_result.ok,
                            "status": tool_lifecycle_status(&tool_result),
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
                if options.cancellation.is_cancelled() {
                    return finish_agent_loop_turn(
                        session,
                        input.task_id,
                        &session_id,
                        &turn_id,
                        &mut events,
                        event_sink,
                        AgentLoopFinish {
                            stop_reason: AgentLoopStopReason::Cancelled,
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
                scheduled_index = batch_end;
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
            diagnostics: stream.diagnostics,
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

#[derive(Clone)]
struct ScheduledAgentLoopToolCall {
    call: AgentLoopToolCall,
    tool_call_id: Option<String>,
    tool_call_name: String,
    execution_mode: ToolExecutionMode,
    timeout_ms: Option<u64>,
}

struct DispatchedAgentLoopToolCall {
    tool_call_id: Option<String>,
    tool_call_name: String,
    started_event_id: AgentEventId,
    result: AgentLoopToolResult,
}

struct ToolBatchDispatchContext<'a> {
    session: &'a ExecutionSession,
    registry: &'a SkillRegistry,
    cancellation: &'a CancellationToken,
    session_id: &'a AgentSessionId,
    turn_id: &'a AgentTurnId,
    events: &'a mut Vec<AgentEvent>,
    event_sink: &'a dyn AgentEventSink,
}

fn schedule_agent_loop_tool_calls(
    calls: Vec<AgentLoopToolCall>,
    definitions: &[AgentLoopToolDefinition],
) -> Vec<ScheduledAgentLoopToolCall> {
    calls
        .into_iter()
        .map(|call| {
            let definition = definitions
                .iter()
                .find(|definition| definition.name == call.name);
            let execution_mode = definition
                .map(|definition| definition.execution_mode)
                .unwrap_or(ToolExecutionMode::Sequential);
            let timeout_ms = definition.and_then(|definition| definition.timeout_ms);
            ScheduledAgentLoopToolCall {
                tool_call_id: call.id.clone(),
                tool_call_name: call.name.clone(),
                call,
                execution_mode,
                timeout_ms,
            }
        })
        .collect()
}

fn scheduled_tool_batch_end(calls: &[ScheduledAgentLoopToolCall], start: usize) -> usize {
    if calls[start].execution_mode == ToolExecutionMode::Sequential {
        return start + 1;
    }
    calls[start..]
        .iter()
        .position(|call| call.execution_mode == ToolExecutionMode::Sequential)
        .map(|offset| start + offset)
        .unwrap_or(calls.len())
        .max(start + 1)
}

async fn dispatch_scheduled_tool_batch(
    context: ToolBatchDispatchContext<'_>,
    iteration: u32,
    batch: Vec<ScheduledAgentLoopToolCall>,
) -> Result<Vec<DispatchedAgentLoopToolCall>> {
    let mut started = Vec::with_capacity(batch.len());
    for scheduled in batch {
        let started_event_id = emit_agent_event(
            context.events,
            context.event_sink,
            context.session_id,
            context.turn_id,
            AgentEventSource::Tool,
            AgentEventKind::ToolCallStarted,
            json!({
                "iteration": iteration,
                "id": &scheduled.tool_call_id,
                "name": &scheduled.tool_call_name,
                "input": redacted_json_value(scheduled.call.input.clone()),
                "execution_mode": scheduled.execution_mode.as_str(),
                "timeout_ms": scheduled.timeout_ms,
            }),
        )?;
        started.push((scheduled, started_event_id));
    }

    if started.len() == 1
        || started
            .iter()
            .any(|(scheduled, _)| scheduled.execution_mode == ToolExecutionMode::Sequential)
    {
        let mut dispatched = Vec::with_capacity(started.len());
        for (scheduled, started_event_id) in started {
            let result = dispatch_scheduled_tool_call_with_cancellation(
                &context,
                iteration,
                scheduled.clone(),
            )
            .await;
            dispatched.push(DispatchedAgentLoopToolCall {
                tool_call_id: scheduled.tool_call_id,
                tool_call_name: scheduled.tool_call_name,
                started_event_id,
                result,
            });
        }
        return Ok(dispatched);
    }

    let (sender, mut receiver) = tokio::sync::mpsc::channel(started.len());
    let mut handles = Vec::with_capacity(started.len());
    let mut pending = Vec::with_capacity(started.len());
    for (scheduled, started_event_id) in started {
        let session = context.session.clone();
        let registry = context.registry.clone();
        let timeout_ms = scheduled.timeout_ms;
        let pending_call = PendingAgentLoopToolCall {
            tool_call_id: scheduled.tool_call_id.clone(),
            tool_call_name: scheduled.tool_call_name.clone(),
            started_event_id,
        };
        pending.push(pending_call.clone());
        let sender = sender.clone();
        handles.push(tokio::spawn(async move {
            let result = dispatch_agent_loop_tool_call(
                &session,
                &registry,
                iteration,
                scheduled.call,
                timeout_ms,
            )
            .await;
            let _ = sender.send((pending_call, result)).await;
        }));
    }
    drop(sender);

    let mut dispatched = Vec::with_capacity(handles.len());
    while !pending.is_empty() {
        tokio::select! {
            received = receiver.recv() => {
                match received {
                    Some((pending_call, result)) => {
                        pending.retain(|call| call.started_event_id != pending_call.started_event_id);
                        dispatched.push(DispatchedAgentLoopToolCall {
                            tool_call_id: pending_call.tool_call_id,
                            tool_call_name: pending_call.tool_call_name,
                            started_event_id: pending_call.started_event_id,
                            result,
                        });
                    }
                    None => {
                        for pending_call in pending.drain(..) {
                            dispatched.push(DispatchedAgentLoopToolCall {
                                tool_call_id: pending_call.tool_call_id,
                                tool_call_name: pending_call.tool_call_name.clone(),
                                started_event_id: pending_call.started_event_id,
                                result: AgentLoopToolResult {
                                    iteration,
                                    name: redact_secrets(&pending_call.tool_call_name),
                                    ok: false,
                                    summary: "tool task ended without reporting a result".into(),
                                    output: json!({"error": "tool task ended without reporting a result"}),
                                },
                            });
                        }
                    }
                }
            }
            _ = context.cancellation.cancelled() => {
                for handle in &handles {
                    handle.abort();
                }
                for pending_call in pending.drain(..) {
                    dispatched.push(DispatchedAgentLoopToolCall {
                        tool_call_id: pending_call.tool_call_id,
                        tool_call_name: pending_call.tool_call_name.clone(),
                        started_event_id: pending_call.started_event_id,
                        result: cancelled_agent_loop_tool_result(
                            iteration,
                            &pending_call.tool_call_name,
                            "tool call cancelled during execution",
                        ),
                    });
                }
                return Ok(dispatched);
            }
        }
    }
    Ok(dispatched)
}

#[derive(Clone)]
struct PendingAgentLoopToolCall {
    tool_call_id: Option<String>,
    tool_call_name: String,
    started_event_id: AgentEventId,
}

async fn dispatch_scheduled_tool_call_with_cancellation(
    context: &ToolBatchDispatchContext<'_>,
    iteration: u32,
    scheduled: ScheduledAgentLoopToolCall,
) -> AgentLoopToolResult {
    tokio::select! {
        result = dispatch_agent_loop_tool_call(
            context.session,
            context.registry,
            iteration,
            scheduled.call,
            scheduled.timeout_ms,
        ) => result,
        _ = context.cancellation.cancelled() => {
            cancelled_agent_loop_tool_result(
                iteration,
                &scheduled.tool_call_name,
                "tool call cancelled during execution",
            )
        }
    }
}

fn cancelled_agent_loop_tool_result(
    iteration: u32,
    tool_call_name: &str,
    summary: &str,
) -> AgentLoopToolResult {
    AgentLoopToolResult {
        iteration,
        name: redact_secrets(tool_call_name),
        ok: false,
        summary: summary.into(),
        output: json!({
            "cancelled": true,
            "reason": "agent loop cancellation requested",
        }),
    }
}

fn emit_cancelled_tool_call_events(
    context: ToolBatchDispatchContext<'_>,
    iteration: u32,
    scheduled_calls: &[ScheduledAgentLoopToolCall],
) -> Result<Vec<AgentLoopToolResult>> {
    let mut results = Vec::with_capacity(scheduled_calls.len());
    for scheduled in scheduled_calls {
        let summary = "tool call cancelled before execution".to_string();
        let output = json!({
            "cancelled": true,
            "reason": "agent loop cancellation requested",
        });
        emit_agent_event(
            context.events,
            context.event_sink,
            context.session_id,
            context.turn_id,
            AgentEventSource::Tool,
            AgentEventKind::ToolCallCancelled,
            json!({
                "iteration": iteration,
                "tool_call_id": &scheduled.tool_call_id,
                "name": &scheduled.tool_call_name,
                "input": redacted_json_value(scheduled.call.input.clone()),
                "execution_mode": scheduled.execution_mode.as_str(),
                "timeout_ms": scheduled.timeout_ms,
                "status": "cancelled",
                "summary": &summary,
                "output": &output,
            }),
        )?;
        results.push(AgentLoopToolResult {
            iteration,
            name: redact_secrets(&scheduled.tool_call_name),
            ok: false,
            summary,
            output,
        });
    }
    Ok(results)
}

fn emit_agent_event(
    events: &mut Vec<AgentEvent>,
    sink: &dyn AgentEventSink,
    session_id: &AgentSessionId,
    turn_id: &AgentTurnId,
    source: AgentEventSource,
    kind: AgentEventKind,
    payload: serde_json::Value,
) -> Result<AgentEventId> {
    let parent_event_id = events.last().map(|event| event.event_id.clone());
    let event = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        parent_event_id,
        source,
        kind,
        payload,
    );
    let event_id = event.event_id.clone();
    sink.emit(&event)?;
    events.push(event);
    Ok(event_id)
}

fn emit_session_approval_record(
    sink: &dyn AgentEventSink,
    session: &ExecutionSession,
    session_id: &AgentSessionId,
    turn_id: &AgentTurnId,
    output: &serde_json::Value,
) -> Result<()> {
    let Some(approval_id) = output
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(());
    };
    let Some(record) = session.approvals.get(approval_id)? else {
        return Ok(());
    };
    let status = match record.status {
        ikaros_harness::ApprovalStatus::Pending => SessionApprovalStatus::Requested,
        ikaros_harness::ApprovalStatus::Approved => SessionApprovalStatus::Approved,
        ikaros_harness::ApprovalStatus::Denied => SessionApprovalStatus::Denied,
        ikaros_harness::ApprovalStatus::Executed => SessionApprovalStatus::Executed,
    };
    let decision = match record.status {
        ikaros_harness::ApprovalStatus::Pending => None,
        _ => Some(redacted_json_value(json!({
            "status": format!("{:?}", record.status),
            "note": record.note,
            "result": record.result,
        }))),
    };
    sink.emit_approval(&SessionApprovalRecord {
        approval_id: approval_id.into(),
        session_id: session_id.clone(),
        turn_id: Some(turn_id.clone()),
        at: OffsetDateTime::now_utc(),
        status,
        request: redacted_json_value(serde_json::to_value(&record.request)?),
        decision,
    })
}

fn redacted_json_value(value: serde_json::Value) -> serde_json::Value {
    redact_json(value)
}

fn tool_lifecycle_end_kind(result: &AgentLoopToolResult) -> AgentEventKind {
    if tool_result_cancelled(result) {
        AgentEventKind::ToolCallCancelled
    } else if result.ok || tool_result_waiting_for_approval(result) {
        AgentEventKind::ToolCallCompleted
    } else {
        AgentEventKind::ToolCallFailed
    }
}

fn tool_lifecycle_status(result: &AgentLoopToolResult) -> &'static str {
    if tool_result_cancelled(result) {
        "cancelled"
    } else if result.ok {
        "completed"
    } else if tool_result_waiting_for_approval(result) {
        "waiting_for_approval"
    } else {
        "failed"
    }
}

fn tool_result_waiting_for_approval(result: &AgentLoopToolResult) -> bool {
    result.output.get("approval_id").is_some()
        || result
            .output
            .get("decision")
            .and_then(serde_json::Value::as_str)
            == Some("ask_user")
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
