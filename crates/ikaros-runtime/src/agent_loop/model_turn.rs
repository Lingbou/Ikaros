// SPDX-License-Identifier: GPL-3.0-only

use super::{
    prompt::{
        agent_loop_system_messages_for_model, agent_loop_tool_definitions,
        build_agent_loop_system_prompt, model_tool_definitions,
    },
    stream::stream_chunks_for_final_content,
    types::{
        AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopInput,
        AgentLoopOptions, AgentLoopReport, AgentLoopStopReason,
    },
};
use ikaros_context::HeuristicTokenEstimator;
use ikaros_core::{IkarosError, Result, redact_secrets};
use ikaros_harness::{AuditEvent, ExecutionSession, GuardrailState, SkillRegistry};
use ikaros_models::{
    ModelMessage, ModelProvider, ModelRequest, ModelStreamEvent, ModelStreamEventSink,
    ProviderRegistry,
};
use ikaros_session::{SessionId, TurnId};
use serde_json::json;
use std::sync::{Arc, Mutex};

mod events;
mod evidence;
mod finish;
mod model_result;
mod provider;
mod state;
mod tool_dispatch;
mod tool_processing;
mod tool_result;

use events::{HookDispatchContext, emit_agent_event, invoke_agent_loop_hook};
use finish::finish_agent_loop_turn;
use model_result::{ModelTurnResultContext, record_agent_loop_model_result};
use provider::request_agent_loop_model_turn;
use state::AgentLoopTurnState;
use tool_dispatch::{
    ToolBatchDispatchContext, emit_cancelled_tool_call_events, schedule_agent_loop_tool_calls,
};
use tool_processing::{ToolProcessingContext, process_scheduled_tool_calls};

pub(super) async fn run_agent_loop_turn(
    input: AgentLoopInput,
    provider: &dyn ModelProvider,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    event_sink: &dyn AgentEventSink,
    options: AgentLoopOptions,
) -> Result<AgentLoopReport> {
    let tool_definitions = agent_loop_tool_definitions(registry, &options.toolsets);
    let prompt_report = build_agent_loop_system_prompt(
        &input.system_prompt,
        &tool_definitions,
        &HeuristicTokenEstimator,
    );
    let prompt_cache = prompt_report.prompt_cache_plan(prompt_cache_policy_for_provider(provider));
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
    let correlation_id = format!("session:{}:turn:{}", session_id.as_str(), turn_id.as_str());
    let correlated_session = session.clone().with_correlation_id(correlation_id.clone());
    let session = &correlated_session;
    let mut events = Vec::new();
    emit_agent_event(
        &mut events,
        event_sink,
        &session_id,
        &turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::SessionStart,
        json!({
            "correlation_id": &correlation_id,
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
            "correlation_id": &correlation_id,
            "task_id": &input.task_id,
            "max_iterations": options.max_iterations,
            "stream": options.stream,
            "tool_count": tool_definitions.len(),
            "prompt": prompt_report.metadata(),
            "prompt_cache": &prompt_cache,
        }),
    )?;
    session.audit.append(
        AuditEvent::new(
            "agent_loop_start",
            None,
            "agent loop started",
            json!({
                "correlation_id": &correlation_id,
                "task_id": &input.task_id,
                "max_iterations": options.max_iterations,
                "stream": options.stream,
                "tool_count": tool_definitions.len(),
                "prompt": prompt_report.metadata(),
                "prompt_cache": &prompt_cache,
                "guardrails": &options.guardrails,
            }),
        )?
        .with_correlation_id(&correlation_id),
    )?;

    let mut messages = agent_loop_system_messages_for_model(
        &prompt_report,
        &options.system_prompt_messages,
        &tool_definitions,
        &HeuristicTokenEstimator,
    )
    .into_iter()
    .map(ModelMessage::system)
    .collect::<Vec<_>>();
    messages.push(ModelMessage::user(redact_secrets(&input.user_input)));
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
    let max_iterations = options.max_iterations.max(1);
    let mut state = AgentLoopTurnState::new(provider.name());

    if options.cancellation.is_cancelled() {
        return finish_agent_loop_turn(
            session,
            input.task_id,
            &session_id,
            &turn_id,
            &mut events,
            event_sink,
            state.finish(AgentLoopStopReason::Cancelled, 0),
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
                state.finish(AgentLoopStopReason::Cancelled, iteration.saturating_sub(1)),
            );
        }
        let request = ModelRequest {
            messages: messages.clone(),
            options: options.request_options.clone(),
            tools: model_tool_definitions(&tool_definitions),
        };
        invoke_agent_loop_hook(
            HookDispatchContext {
                hooks: options.hooks(),
                events: &mut events,
                event_sink,
                session_id: &session_id,
                turn_id: &turn_id,
                task_id: input.task_id.as_deref(),
                iteration,
                event_id: None,
                hook_name: "before_provider_request",
            },
            json!({
                "provider": provider.name(),
                "stream": options.stream,
                "message_count": request.messages.len(),
                "tool_count": request.tools.len(),
                "max_tokens": request.options.max_tokens,
                "temperature": request.options.temperature,
            }),
            |hooks, event| hooks.before_provider_request(event),
        )?;
        let live_stream_events = Arc::new(Mutex::new(Vec::new()));
        let turn_result = {
            let mut stream_event_sink = options.stream.then(|| LiveAgentLoopStreamEventSink {
                events: live_stream_events.clone(),
                event_sink,
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                iteration,
            });
            let stream_event_sink = stream_event_sink
                .as_mut()
                .map(|sink| sink as &mut dyn ModelStreamEventSink);
            tokio::select! {
                _ = options.cancellation.cancelled() => {
                    return finish_agent_loop_turn(
                        session,
                        input.task_id,
                        &session_id,
                        &turn_id,
                        &mut events,
                        event_sink,
                        state.finish(AgentLoopStopReason::Cancelled, iteration.saturating_sub(1)),
                    );
                }
                result = request_agent_loop_model_turn(
                    provider,
                    request,
                    options.stream,
                    stream_event_sink,
                ) => result
            }
        };
        let live_events = live_stream_events
            .lock()
            .map_err(|_| IkarosError::Message("agent loop stream event lock is poisoned".into()))?
            .clone();
        events.extend(live_events);
        let turn = match turn_result {
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
                        "tool_result_count": state.tool_results.len(),
                        "status": "failed",
                    }),
                )?;
                return Err(error);
            }
        };
        let result = record_agent_loop_model_result(
            ModelTurnResultContext {
                session,
                events: &mut events,
                event_sink,
                session_id: &session_id,
                turn_id: &turn_id,
                task_id: input.task_id.as_deref(),
                correlation_id: &correlation_id,
                hooks: options.hooks(),
                iteration,
            },
            &turn,
            &mut state,
        )?;
        let response = turn.response;

        if let Some(envelope) = result.envelope {
            state.last_content = envelope
                .final_answer
                .clone()
                .unwrap_or_else(|| response.content.clone());
            if envelope.tool_calls.is_empty() {
                if turn.streamed {
                    state.final_streamed = true;
                    state.final_stream_chunks =
                        stream_chunks_for_final_content(&turn.stream_chunks, &state.last_content);
                }
                return finish_agent_loop_turn(
                    session,
                    input.task_id,
                    &session_id,
                    &turn_id,
                    &mut events,
                    event_sink,
                    state.finish(AgentLoopStopReason::FinalAnswer, iteration),
                );
            }
            messages.push(ModelMessage::assistant_with_tool_calls(
                redact_secrets(&response.content),
                response.tool_calls.clone(),
            ));
            let scheduled_calls =
                schedule_agent_loop_tool_calls(envelope.tool_calls, &tool_definitions);
            if options.cancellation.is_cancelled() {
                state.tool_results.extend(emit_cancelled_tool_call_events(
                    ToolBatchDispatchContext {
                        session,
                        registry,
                        cancellation: &options.cancellation,
                        hooks: options.hooks(),
                        task_id: input.task_id.as_deref(),
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
                    state.finish(AgentLoopStopReason::Cancelled, iteration),
                );
            }
            let stop_reason = process_scheduled_tool_calls(
                ToolProcessingContext {
                    session,
                    registry,
                    cancellation: &options.cancellation,
                    guardrails: &options.guardrails,
                    hooks: options.hooks(),
                    task_id: input.task_id.as_deref(),
                    session_id: &session_id,
                    turn_id: &turn_id,
                    events: &mut events,
                    event_sink,
                },
                iteration,
                scheduled_calls,
                &mut state,
                &mut guardrails,
                &mut messages,
            )
            .await?;
            if let Some(stop_reason) = stop_reason {
                return finish_agent_loop_turn(
                    session,
                    input.task_id,
                    &session_id,
                    &turn_id,
                    &mut events,
                    event_sink,
                    state.finish(stop_reason, iteration),
                );
            }
            continue;
        }

        state.last_content = response.content;
        if turn.streamed {
            state.final_streamed = true;
            state.final_stream_chunks =
                stream_chunks_for_final_content(&turn.stream_chunks, &state.last_content);
        }
        return finish_agent_loop_turn(
            session,
            input.task_id,
            &session_id,
            &turn_id,
            &mut events,
            event_sink,
            state.finish(AgentLoopStopReason::FinalAnswer, iteration),
        );
    }

    finish_agent_loop_turn(
        session,
        input.task_id,
        &session_id,
        &turn_id,
        &mut events,
        event_sink,
        state.finish(AgentLoopStopReason::IterationBudget, max_iterations),
    )
}

struct LiveAgentLoopStreamEventSink<'a> {
    events: Arc<Mutex<Vec<AgentEvent>>>,
    event_sink: &'a dyn AgentEventSink,
    session_id: SessionId,
    turn_id: TurnId,
    iteration: u32,
}

impl ModelStreamEventSink for LiveAgentLoopStreamEventSink<'_> {
    fn emit(&mut self, event: ModelStreamEvent) -> Result<()> {
        let mut events = self
            .events
            .lock()
            .map_err(|_| IkarosError::Message("agent loop stream event lock is poisoned".into()))?;
        let parent_event_id = events.last().map(|event| event.event_id.clone());
        let event = AgentEvent::new(
            self.session_id.clone(),
            self.turn_id.clone(),
            parent_event_id,
            AgentEventSource::Model,
            AgentEventKind::ModelStream(event),
            json!({
                "iteration": self.iteration,
            }),
        );
        self.event_sink.emit(&event)?;
        events.push(event);
        Ok(())
    }
}

fn prompt_cache_policy_for_provider(provider: &dyn ModelProvider) -> String {
    ProviderRegistry
        .descriptor(provider.name(), "", provider.model_id())
        .map(|descriptor| descriptor.profile_policy.prompt_cache)
        .unwrap_or_else(|_| "none".into())
}
