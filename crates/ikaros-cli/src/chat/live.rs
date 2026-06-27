// SPDX-License-Identifier: GPL-3.0-only

use super::{
    interactive::InteractiveChatRuntime, interactive_chat_turn_error_actions,
    interactive_chat_turn_error_kind, workbench,
};
use anyhow::Result;
use ikaros_core::{IkarosError, redact_secrets};
use ikaros_models::ModelStreamEvent;
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, SessionId, SessionStore,
    SqliteSessionStore, TurnId,
};
use std::{
    env,
    sync::{Arc, Mutex},
};

pub(super) fn print_live_event_cells(
    runtime: &InteractiveChatRuntime,
    turn_id: &TurnId,
) -> Result<()> {
    if runtime.fullscreen_stdout_quiet() {
        return Ok(());
    }
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let Some(replay) = store.replay_session(&session_id)? else {
        println!("live_cells: 0");
        return Ok(());
    };
    let events = replay
        .agent_events
        .iter()
        .filter(|event| event.turn_id.as_str() == turn_id.as_str())
        .collect::<Vec<_>>();
    let visible_events = events
        .iter()
        .copied()
        .filter(|event| default_live_cell_event(&event.kind))
        .collect::<Vec<_>>();
    let cells = compact_live_event_cells(&events);
    println!(
        "live_cells: compact_cells={} total_events={} visible_events={} detail={}",
        cells.len(),
        events.len(),
        visible_events.len(),
        if verbose_live_cells_enabled() {
            "expanded"
        } else {
            "json"
        }
    );
    println!("{}", live_cell_summary_line(&events, &visible_events));
    println!("{}", live_cells_json_line(&events, &visible_events, &cells));
    if verbose_live_cells_enabled() {
        for cell in &cells {
            println!("- {}", cell.render());
        }
    }
    Ok(())
}

fn verbose_live_cells_enabled() -> bool {
    env::var_os("IKAROS_WORKBENCH_VERBOSE_LIVE_CELLS").is_some()
}

#[derive(Clone, Default)]
pub(super) struct WorkbenchLiveEventSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
    snapshots: Arc<Mutex<Vec<String>>>,
    stdout_updates: bool,
}

impl WorkbenchLiveEventSink {
    pub(super) fn with_stdout_updates(stdout_updates: bool) -> Self {
        Self {
            stdout_updates,
            ..Self::default()
        }
    }

    fn has_error_event_for_turn(&self, turn_id: &TurnId) -> ikaros_core::Result<bool> {
        self.events
            .lock()
            .map_err(|_| IkarosError::Message("workbench live event lock is poisoned".into()))
            .map(|events| {
                events.iter().any(|event| {
                    event.turn_id.as_str() == turn_id.as_str()
                        && matches!(event.kind, AgentEventKind::Error)
                })
            })
    }

    #[cfg(test)]
    pub(super) fn snapshots(&self) -> ikaros_core::Result<Vec<String>> {
        self.snapshots
            .lock()
            .map(|snapshots| snapshots.clone())
            .map_err(|_| IkarosError::Message("workbench live snapshot lock is poisoned".into()))
    }

    pub(super) fn events(&self) -> ikaros_core::Result<Vec<AgentEvent>> {
        self.events
            .lock()
            .map(|events| events.clone())
            .map_err(|_| IkarosError::Message("workbench live event lock is poisoned".into()))
    }
}

pub(super) fn emit_interactive_chat_turn_failure_evidence(
    sink: &dyn AgentEventSink,
    live_sink: &WorkbenchLiveEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    error: &anyhow::Error,
) -> ikaros_core::Result<()> {
    if live_sink.has_error_event_for_turn(turn_id)? {
        return Ok(());
    }
    let message = error.to_string();
    let error_kind = interactive_chat_turn_error_kind(&message);
    sink.emit(&AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::Error,
        serde_json::json!({
            "phase": "interactive_chat_turn",
            "message": redact_secrets(&message),
            "error_kind": error_kind,
            "recoverable": error_kind != "unknown",
            "actions": interactive_chat_turn_error_actions(error_kind, &message),
        }),
    ))?;
    sink.emit(&AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        serde_json::json!({
            "status": "failed",
            "phase": "interactive_chat_turn",
            "error_kind": error_kind,
        }),
    ))
}

impl AgentEventSink for WorkbenchLiveEventSink {
    fn emit(&self, event: &AgentEvent) -> ikaros_core::Result<()> {
        let snapshot = {
            let mut events = self.events.lock().map_err(|_| {
                IkarosError::Message("workbench live event lock is poisoned".into())
            })?;
            events.push(event.clone());
            live_cell_snapshot(&events)
        };
        self.snapshots
            .lock()
            .map_err(|_| IkarosError::Message("workbench live snapshot lock is poisoned".into()))?
            .push(snapshot.clone());
        if self.stdout_updates {
            println!("live_cell_update:");
            for line in snapshot.lines() {
                println!("{line}");
            }
        }
        Ok(())
    }
}

fn live_cell_snapshot(events: &[AgentEvent]) -> String {
    let event_refs = events.iter().collect::<Vec<_>>();
    let visible_events = event_refs
        .iter()
        .copied()
        .filter(|event| default_live_cell_event(&event.kind))
        .collect::<Vec<_>>();
    let cells = compact_live_event_cells(&event_refs);
    let mut lines = vec![
        format!("live_cells: {} total_events={}", cells.len(), events.len()),
        live_cell_summary_line(&event_refs, &visible_events),
        live_cells_json_line(&event_refs, &visible_events, &cells),
    ];
    lines.extend(cells.iter().map(|cell| format!("- {}", cell.render())));
    lines.join("\n")
}

pub(super) fn compact_live_event_cells(events: &[&AgentEvent]) -> Vec<workbench::WorkbenchCell> {
    let mut cells = Vec::new();
    if let Some(cell) = model_stream_summary_cell(events) {
        cells.push(cell);
    }
    if let Some(cell) = tool_progress_summary_cell(events) {
        cells.push(cell);
    }
    if let Some(cell) = context_progress_summary_cell(events) {
        cells.push(cell);
    }
    let visible_events = events
        .iter()
        .copied()
        .filter(|event| default_live_cell_event(&event.kind))
        .collect::<Vec<_>>();
    let remaining = 8usize.saturating_sub(cells.len());
    let start = visible_events.len().saturating_sub(remaining);
    for event in &visible_events[start..] {
        cells.push(workbench::agent_event_cell(event));
    }
    cells
}

fn model_stream_summary_cell(events: &[&AgentEvent]) -> Option<workbench::WorkbenchCell> {
    let mut provider = "unknown".to_owned();
    let mut model = "unknown".to_owned();
    let mut text_delta_chunks = 0usize;
    let mut reasoning_delta_chunks = 0usize;
    let mut refusal_delta_chunks = 0usize;
    let mut tool_call_events = 0usize;
    let mut errors = 0usize;
    let mut usage_total = None;
    let mut done = false;
    let mut seen = false;

    for event in events {
        let AgentEventKind::ModelStream(stream_event) = &event.kind else {
            continue;
        };
        seen = true;
        match stream_event {
            ModelStreamEvent::Start {
                provider: event_provider,
                model: event_model,
            } => {
                provider = workbench::terminal_inline(event_provider);
                model = workbench::terminal_inline(event_model);
            }
            ModelStreamEvent::TextDelta(_) => text_delta_chunks += 1,
            ModelStreamEvent::ReasoningDelta(_) => reasoning_delta_chunks += 1,
            ModelStreamEvent::RefusalDelta(_) => refusal_delta_chunks += 1,
            ModelStreamEvent::ToolCallStart { .. }
            | ModelStreamEvent::ToolCallDelta { .. }
            | ModelStreamEvent::ToolCallEnd { .. } => tool_call_events += 1,
            ModelStreamEvent::Usage(usage) => {
                usage_total = Some(usage.total_or_prompt_completion());
            }
            ModelStreamEvent::Error { .. } => errors += 1,
            ModelStreamEvent::Done => done = true,
        }
    }

    seen.then(|| workbench::WorkbenchCell {
        kind: workbench::WorkbenchCellKind::Model,
        title: "model stream summary".into(),
        detail: format!(
            "provider={} model={} text_delta_chunks={} reasoning_delta_chunks={} refusal_delta_chunks={} tool_call_events={} usage_total={} done={} errors={}",
            provider,
            model,
            text_delta_chunks,
            reasoning_delta_chunks,
            refusal_delta_chunks,
            tool_call_events,
            usage_total
                .map(|tokens| tokens.to_string())
                .unwrap_or_else(|| "unknown".into()),
            done,
            errors
        ),
    })
}

fn tool_progress_summary_cell(events: &[&AgentEvent]) -> Option<workbench::WorkbenchCell> {
    let mut started = 0usize;
    let mut output = 0usize;
    let mut completed = 0usize;
    let mut failed = 0usize;
    let mut cancelled = 0usize;
    let mut latest_tool = "unknown".to_owned();
    let mut latest_status = "unknown".to_owned();

    for event in events {
        let status = match &event.kind {
            AgentEventKind::ToolCallStarted => {
                started += 1;
                "started"
            }
            AgentEventKind::ToolCallOutputDelta => {
                output += 1;
                "output"
            }
            AgentEventKind::ToolCallCompleted => {
                completed += 1;
                "completed"
            }
            AgentEventKind::ToolCallFailed => {
                failed += 1;
                "failed"
            }
            AgentEventKind::ToolCallCancelled => {
                cancelled += 1;
                "cancelled"
            }
            _ => continue,
        };
        latest_status = event
            .payload
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(status)
            .to_owned();
        latest_tool = event
            .payload
            .get("name")
            .and_then(serde_json::Value::as_str)
            .map(workbench::terminal_inline)
            .unwrap_or_else(|| "unknown".into());
    }

    let total = started + output + completed + failed + cancelled;
    (total > 0).then(|| workbench::WorkbenchCell {
        kind: workbench::WorkbenchCellKind::Tool,
        title: "tool progress summary".into(),
        detail: format!(
            "started={} output={} completed={} failed={} cancelled={} latest_tool={} latest_status={} trace=/trace --kind tool tools=/tools",
            started,
            output,
            completed,
            failed,
            cancelled,
            latest_tool,
            workbench::terminal_inline(&latest_status)
        ),
    })
}

fn context_progress_summary_cell(events: &[&AgentEvent]) -> Option<workbench::WorkbenchCell> {
    let mut diffs = 0usize;
    let mut compacted = 0usize;
    let mut latest_sections = 0usize;
    let mut latest_references = 0usize;
    let mut latest_used = None;
    let mut latest_max = None;
    let mut latest_window = None;
    let mut latest_estimator = "unknown".to_owned();

    for event in events {
        match &event.kind {
            AgentEventKind::ContextDiff => {
                diffs += 1;
                latest_sections = json_array_len(&event.payload, "sections");
                latest_references = json_array_len(&event.payload, "references");
                if let Some(budget) = event.payload.get("budget") {
                    latest_used = json_u64(budget, "used_tokens");
                    latest_max = json_u64(budget, "max_tokens");
                    latest_window = json_u64(budget, "context_window");
                    latest_estimator = budget
                        .get("estimator")
                        .and_then(serde_json::Value::as_str)
                        .map(workbench::terminal_inline)
                        .unwrap_or_else(|| "unknown".into());
                }
            }
            AgentEventKind::ContextCompacted => {
                compacted += 1;
                latest_sections = json_array_len(&event.payload, "compressed_sections");
            }
            _ => continue,
        }
    }

    let total = diffs + compacted;
    (total > 0).then(|| workbench::WorkbenchCell {
        kind: workbench::WorkbenchCellKind::Context,
        title: "context progress summary".into(),
        detail: format!(
            "diffs={} compacted={} latest_sections={} latest_references={} used={} max={} context_window={} estimator={} trace=/trace --kind context context=/context",
            diffs,
            compacted,
            latest_sections,
            latest_references,
            latest_used
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            latest_max
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            latest_window
                .map(|value| value.to_string())
                .unwrap_or_else(|| "unknown".into()),
            latest_estimator,
        ),
    })
}

fn live_cell_summary_line(events: &[&AgentEvent], visible_events: &[&AgentEvent]) -> String {
    let model_stream_suppressed = events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::ModelStream(_)))
        .count();
    format!(
        "live_cell_summary: session={} model={} tool={} context={} memory={} coding={} approval={} continuation={} audit={} error={} model_stream_suppressed={}",
        count_live_events(visible_events, "session"),
        count_live_events(events, "model"),
        count_live_events(visible_events, "tool"),
        count_live_events(visible_events, "context"),
        count_live_events(visible_events, "memory"),
        count_live_events(visible_events, "coding"),
        count_live_events(visible_events, "approval"),
        count_live_events(visible_events, "continuation"),
        count_live_events(visible_events, "audit"),
        count_live_events(visible_events, "error"),
        model_stream_suppressed
    )
}

pub(super) fn live_cells_json_line(
    events: &[&AgentEvent],
    visible_events: &[&AgentEvent],
    cells: &[workbench::WorkbenchCell],
) -> String {
    format!(
        "live_cells_json: {}",
        serde_json::json!({
            "schema": "ikaros-workbench-live-cells-v1",
            "version": 1,
            "total_events": events.len(),
            "visible_events": visible_events.len(),
            "model_stream_suppressed": events
                .iter()
                .filter(|event| matches!(event.kind, AgentEventKind::ModelStream(_)))
                .count(),
            "counts": {
                "session": count_live_events(visible_events, "session"),
                "model": count_live_events(events, "model"),
                "tool": count_live_events(visible_events, "tool"),
                "context": count_live_events(visible_events, "context"),
                "memory": count_live_events(visible_events, "memory"),
                "coding": count_live_events(visible_events, "coding"),
                "approval": count_live_events(visible_events, "approval"),
                "continuation": count_live_events(visible_events, "continuation"),
                "audit": count_live_events(visible_events, "audit"),
                "error": count_live_events(visible_events, "error"),
            },
            "cells": cells.iter().map(live_cell_json).collect::<Vec<_>>(),
        })
    )
}

fn live_cell_json(cell: &workbench::WorkbenchCell) -> serde_json::Value {
    serde_json::json!({
        "kind": cell.kind.as_str(),
        "title": workbench::terminal_inline(&cell.title),
        "detail": workbench::terminal_inline(&cell.detail),
    })
}

fn count_live_events(events: &[&AgentEvent], category: &str) -> usize {
    events
        .iter()
        .filter(|event| live_event_category(&event.kind) == category)
        .count()
}

fn live_event_category(kind: &AgentEventKind) -> &'static str {
    match kind {
        AgentEventKind::ModelStream(_) | AgentEventKind::ModelDiagnostic(_) => "model",
        AgentEventKind::ToolCallStarted
        | AgentEventKind::ToolCallOutputDelta
        | AgentEventKind::ToolCallCompleted
        | AgentEventKind::ToolCallFailed
        | AgentEventKind::ToolCallCancelled => "tool",
        AgentEventKind::ContextDiff | AgentEventKind::ContextCompacted => "context",
        AgentEventKind::MemoryLifecycle => "memory",
        AgentEventKind::CodingTurn => "coding",
        AgentEventKind::AuditAnchor => "audit",
        AgentEventKind::ContinuationStarted
        | AgentEventKind::ContinuationCompleted
        | AgentEventKind::ContinuationFailed
        | AgentEventKind::ContinuationCancelled => "continuation",
        AgentEventKind::ApprovalRequested | AgentEventKind::ApprovalResolved => "approval",
        AgentEventKind::Error => "error",
        AgentEventKind::SessionStart
        | AgentEventKind::TurnStart
        | AgentEventKind::UserMessage
        | AgentEventKind::TurnEnd => "session",
    }
}

fn json_u64(payload: &serde_json::Value, key: &str) -> Option<u64> {
    payload.get(key).and_then(serde_json::Value::as_u64)
}

fn json_array_len(payload: &serde_json::Value, key: &str) -> usize {
    payload
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(Vec::len)
        .unwrap_or(0)
}

pub(super) fn default_live_cell_event(kind: &AgentEventKind) -> bool {
    !matches!(kind, AgentEventKind::ModelStream(_))
}
