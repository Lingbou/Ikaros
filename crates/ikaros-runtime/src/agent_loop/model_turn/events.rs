// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::types::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopHookEvent,
    AgentLoopHooks, AgentSessionId, AgentTurnId,
};
use ikaros_core::{Result, redact_json, redact_secrets};
use ikaros_session::AgentEventId;
use serde_json::json;

pub(super) struct HookDispatchContext<'a> {
    pub(super) hooks: &'a dyn AgentLoopHooks,
    pub(super) events: &'a mut Vec<AgentEvent>,
    pub(super) event_sink: &'a dyn AgentEventSink,
    pub(super) session_id: &'a AgentSessionId,
    pub(super) turn_id: &'a AgentTurnId,
    pub(super) task_id: Option<&'a str>,
    pub(super) iteration: u32,
    pub(super) event_id: Option<&'a AgentEventId>,
    pub(super) hook_name: &'static str,
}

pub(super) fn invoke_agent_loop_hook(
    context: HookDispatchContext<'_>,
    payload: serde_json::Value,
    invoke: impl FnOnce(&dyn AgentLoopHooks, &AgentLoopHookEvent) -> Result<()>,
) -> Result<()> {
    let hook_event = AgentLoopHookEvent {
        session_id: context.session_id.clone(),
        turn_id: context.turn_id.clone(),
        task_id: context.task_id.map(ToOwned::to_owned),
        iteration: context.iteration,
        event_id: context.event_id.cloned(),
        payload: redacted_json_value(payload),
    };
    if let Err(error) = invoke(context.hooks, &hook_event) {
        emit_agent_event(
            context.events,
            context.event_sink,
            context.session_id,
            context.turn_id,
            AgentEventSource::Runtime,
            AgentEventKind::Error,
            json!({
                "phase": "agent_loop_hook",
                "hook": context.hook_name,
                "iteration": context.iteration,
                "message": redact_secrets(&error.to_string()),
            }),
        )?;
    }
    Ok(())
}

pub(super) fn emit_agent_event(
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

pub(super) fn redacted_json_value(value: serde_json::Value) -> serde_json::Value {
    redact_json(value)
}
