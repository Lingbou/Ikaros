// SPDX-License-Identifier: GPL-3.0-only

use crate::AgentEventSink;
use ikaros_core::{Result, redact_secrets};
use ikaros_memory::MemoryLifecycleReport;
use ikaros_session::{AgentEvent, AgentEventKind, AgentEventSource, SessionId, TurnId};
use serde_json::json;

pub(super) fn emit_chat_event(
    events: &mut Vec<AgentEvent>,
    sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
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

pub(super) fn emit_chat_lifecycle_event(
    sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    source: AgentEventSource,
    kind: AgentEventKind,
    payload: serde_json::Value,
) -> Result<()> {
    sink.emit(&AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        source,
        kind,
        payload,
    ))
}

pub(super) fn emit_memory_lifecycle_report(
    sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    agent_id: &str,
    chat_session_id: &str,
    report: &MemoryLifecycleReport,
) -> Result<()> {
    emit_chat_lifecycle_event(
        sink,
        session_id,
        turn_id,
        AgentEventSource::Memory,
        AgentEventKind::MemoryLifecycle,
        json!({
            "phase": &report.phase,
            "status": "ok",
            "agent_id": agent_id,
            "session_id": chat_session_id,
            "records_read": report.records_read,
            "records_written": report.records_written,
            "source_ref": &report.source_ref,
            "notes": &report.notes,
            "report": report,
        }),
    )
}

pub(super) fn emit_chat_failure_event(
    sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    phase: &str,
    error: &dyn std::fmt::Display,
) -> Result<()> {
    let mut events = Vec::new();
    emit_chat_failure_events(&mut events, sink, session_id, turn_id, phase, error)
}

pub(super) fn emit_chat_failure_events(
    events: &mut Vec<AgentEvent>,
    sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    phase: &str,
    error: &dyn std::fmt::Display,
) -> Result<()> {
    emit_chat_event(
        events,
        sink,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::Error,
        json!({
            "phase": phase,
            "message": redact_secrets(&error.to_string()),
        }),
    )?;
    emit_chat_event(
        events,
        sink,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "status": "failed",
            "phase": phase,
        }),
    )
}
