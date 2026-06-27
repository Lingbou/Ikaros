// SPDX-License-Identifier: GPL-3.0-only

use super::{
    events::{HookDispatchContext, emit_agent_event, invoke_agent_loop_hook},
    finish::merge_token_usage,
    state::AgentLoopTurnState,
};
use crate::agent_loop::{
    tool_parse::{agent_loop_model_envelope_from_response, agent_loop_tool_call_diagnostic},
    types::{
        AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopHooks,
        AgentLoopModelEnvelope, AgentLoopModelTurn,
    },
};
use ikaros_core::{Result, redact_secrets};
use ikaros_harness::{AuditEvent, ExecutionSession};
use ikaros_models::ModelRequestDiagnostic;
use ikaros_session::{SessionId, TurnId};
use serde_json::json;

pub(super) struct ModelTurnResultContext<'a> {
    pub session: &'a ExecutionSession,
    pub events: &'a mut Vec<AgentEvent>,
    pub event_sink: &'a dyn AgentEventSink,
    pub session_id: &'a SessionId,
    pub turn_id: &'a TurnId,
    pub task_id: Option<&'a str>,
    pub correlation_id: &'a str,
    pub hooks: &'a dyn AgentLoopHooks,
    pub iteration: u32,
}

pub(super) struct ModelTurnResultRecord {
    pub envelope: Option<AgentLoopModelEnvelope>,
}

pub(super) fn record_agent_loop_model_result(
    ctx: ModelTurnResultContext<'_>,
    turn: &AgentLoopModelTurn,
    state: &mut AgentLoopTurnState,
) -> Result<ModelTurnResultRecord> {
    let response = &turn.response;
    invoke_agent_loop_hook(
        HookDispatchContext {
            hooks: ctx.hooks,
            events: &mut *ctx.events,
            event_sink: ctx.event_sink,
            session_id: ctx.session_id,
            turn_id: ctx.turn_id,
            task_id: ctx.task_id,
            iteration: ctx.iteration,
            event_id: None,
            hook_name: "after_provider_response",
        },
        json!({
            "provider": &response.provider,
            "model": &response.model,
            "streamed": turn.streamed,
            "stream_event_count": turn.stream_events.len(),
            "stream_chunk_count": turn.stream_chunks.len(),
            "native_tool_call_count": response.tool_calls.len(),
            "usage": &response.usage,
            "diagnostic_count": response.diagnostics.len(),
        }),
        |hooks, event| hooks.after_provider_response(event),
    )?;
    if !turn.stream_events_already_emitted {
        for event in &turn.stream_events {
            emit_agent_event(
                &mut *ctx.events,
                ctx.event_sink,
                ctx.session_id,
                ctx.turn_id,
                AgentEventSource::Model,
                AgentEventKind::ModelStream(event.clone()),
                json!({
                    "iteration": ctx.iteration,
                }),
            )?;
        }
    }
    let sanitized_model_diagnostics: Vec<ModelRequestDiagnostic> = response
        .diagnostics
        .iter()
        .cloned()
        .map(ModelRequestDiagnostic::sanitized)
        .collect();
    let diagnostic_kinds = sanitized_model_diagnostics
        .iter()
        .map(|diagnostic| redact_secrets(&diagnostic.kind))
        .collect::<Vec<_>>()
        .join(",");
    tracing::info!(
        target: "ikaros.runtime",
        event = "agent_loop_model_result",
        session_id = %ctx.session_id,
        turn_id = %ctx.turn_id,
        correlation_id = %ctx.correlation_id,
        task_id = %ctx.task_id.unwrap_or_default(),
        iteration = ctx.iteration,
        provider = %redact_secrets(&response.provider),
        model = %redact_secrets(&response.model),
        streamed = turn.streamed,
        native_tool_call_count = response.tool_calls.len(),
        stream_event_count = turn.stream_events.len(),
        diagnostic_count = sanitized_model_diagnostics.len(),
        diagnostic_kinds = %diagnostic_kinds,
    );
    for diagnostic in sanitized_model_diagnostics.iter().cloned() {
        let diag_kind = diagnostic.kind.clone();
        emit_agent_event(
            &mut *ctx.events,
            ctx.event_sink,
            ctx.session_id,
            ctx.turn_id,
            AgentEventSource::Model,
            AgentEventKind::ModelDiagnostic(diagnostic),
            json!({
                "iteration": ctx.iteration,
                "diagnostic_kind": diag_kind,
            }),
        )?;
    }
    state.last_provider = response.provider.clone();
    state.last_model = response.model.clone();
    let total_usage = std::mem::take(&mut state.total_usage);
    state.total_usage = merge_token_usage(total_usage, &response.usage);
    let envelope = agent_loop_model_envelope_from_response(response);
    let diagnostic = agent_loop_tool_call_diagnostic(ctx.iteration, response, envelope.as_ref());
    let tool_call_count = envelope
        .as_ref()
        .map(|envelope| envelope.tool_calls.len())
        .unwrap_or_default();
    let final_answer = envelope
        .as_ref()
        .and_then(|envelope| envelope.final_answer.clone());
    ctx.session.audit.append(
        AuditEvent::new(
            "agent_loop_model_result",
            None,
            "agent loop model result",
            json!({
                "correlation_id": ctx.correlation_id,
                "task_id": ctx.task_id,
                "iteration": ctx.iteration,
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
                "diagnostics": &sanitized_model_diagnostics,
            }),
        )?
        .with_correlation_id(ctx.correlation_id),
    )?;
    state.tool_call_diagnostics.push(diagnostic);
    Ok(ModelTurnResultRecord { envelope })
}
