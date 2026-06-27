// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) fn debug_trace(
    args: DebugSessionQuery,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (state_db, replay) = replay_session(paths, workspace, agent_override, &args.session_id)?;
    let events = filter_turn_events(
        &replay.agent_events,
        &args.session_id,
        args.turn_id.as_deref(),
    )?;
    let mut event_counts = BTreeMap::<String, usize>::new();
    for event in &events {
        *event_counts
            .entry(trace_category(event).to_owned())
            .or_default() += 1;
    }
    let mut turn_spans = BTreeMap::<String, TraceTurnSpan>::new();
    for event in &events {
        let turn_id = event.turn_id.to_string();
        let span = turn_spans
            .entry(turn_id.clone())
            .or_insert_with(|| TraceTurnSpan::new(turn_id));
        span.observe_event(event);
    }
    for entry in &replay.entries {
        if args
            .turn_id
            .as_deref()
            .is_some_and(|turn_id| entry.turn_id.as_ref().map(|id| id.as_str()) != Some(turn_id))
        {
            continue;
        }
        if let Some(turn_id) = entry.turn_id.as_ref() {
            let span = turn_spans
                .entry(turn_id.to_string())
                .or_insert_with(|| TraceTurnSpan::new(turn_id.to_string()));
            span.entry_count += 1;
        }
    }
    for approval in &replay.approvals {
        if args
            .turn_id
            .as_deref()
            .is_some_and(|turn_id| approval.turn_id.as_ref().map(|id| id.as_str()) != Some(turn_id))
        {
            continue;
        }
        if let Some(turn_id) = approval.turn_id.as_ref() {
            let span = turn_spans
                .entry(turn_id.to_string())
                .or_insert_with(|| TraceTurnSpan::new(turn_id.to_string()));
            span.approval_count += 1;
        }
    }
    if let Some(turn_id) = args.turn_id.as_deref()
        && turn_spans.is_empty()
        && !replay_contains_turn(&replay, turn_id)
    {
        return Err(anyhow!(
            "turn not found in session {}: {turn_id}",
            args.session_id
        ));
    }
    let turn_spans = turn_spans
        .into_values()
        .map(|span| span.into_json(&args.session_id))
        .collect::<Vec<_>>();
    let protocol_events = events
        .iter()
        .map(|event| (*event).clone())
        .collect::<Vec<_>>();
    let state_trace_entries =
        agent_events_to_state_trace(&protocol_events, args.turn_id.as_deref());
    let summarized_state_trace_entries = state_trace_entries
        .iter()
        .map(summarized_state_trace_entry)
        .collect::<Vec<_>>();
    let turn_state_snapshots =
        protocol_turn_state_snapshots(&args.session_id, &summarized_state_trace_entries);
    let state_trace = summarized_state_trace_entries
        .iter()
        .map(trace_state_entry_summary)
        .collect::<Vec<_>>();
    let ordered_events = events
        .into_iter()
        .map(trace_event_summary)
        .collect::<Vec<_>>();
    let output = json!({
        "format": "ikaros-trace-v1",
        "protocol": {
            "name": IKAROS_PROTOCOL_NAME,
            "version": IKAROS_PROTOCOL_VERSION,
        },
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "state_db": state_db.display().to_string(),
        "session_source": replay.session.source,
        "event_counts": event_counts,
        "turn_spans": turn_spans,
        "turn_state_snapshots": turn_state_snapshots,
        "state_trace": state_trace,
        "ordered_events": ordered_events,
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

pub(in crate::debug) fn summarized_state_trace_entry(entry: &StateTraceEntry) -> StateTraceEntry {
    let mut summarized = entry.clone();
    summarized.payload = trace_payload_summary(&entry.payload);
    summarized
}

pub(in crate::debug) fn protocol_turn_state_snapshots(
    session_id: &str,
    state_trace: &[StateTraceEntry],
) -> Vec<TurnStateSnapshot> {
    let mut by_turn = BTreeMap::<String, Vec<StateTraceEntry>>::new();
    for entry in state_trace {
        by_turn
            .entry(entry.turn_id.clone())
            .or_default()
            .push(entry.clone());
    }
    by_turn
        .into_iter()
        .map(|(turn_id, trace)| TurnStateSnapshot::from_trace(session_id, turn_id, trace))
        .collect()
}

pub(in crate::debug) fn trace_state_entry_summary(entry: &StateTraceEntry) -> Value {
    json!({
        "protocol_version": entry.protocol_version,
        "session_id": entry.session_id,
        "turn_id": entry.turn_id,
        "event_id": entry.event_id,
        "correlation_id": entry.correlation_id,
        "at": entry.at,
        "source": entry.source,
        "category": entry.category,
        "event_kind": entry.event_kind,
        "state_before": entry.state_before,
        "state_after": entry.state_after,
        "title": entry.title,
        "detail": entry.detail,
        "waiting_on": entry.waiting_on,
        "stop_reason": entry.stop_reason,
        "error": entry.error,
        "payload": trace_payload_summary(&entry.payload),
    })
}

pub(in crate::debug) fn trace_payload_summary(payload: &Value) -> Value {
    json!({
        "kind": payload.get("kind").and_then(Value::as_str),
        "phase": payload.get("phase").and_then(Value::as_str),
        "status": payload.get("status").and_then(Value::as_str),
        "error_kind": payload.get("error_kind").and_then(Value::as_str),
        "model_stream_kind": payload.get("model_stream_kind").and_then(Value::as_str),
        "diagnostic_kind": payload.get("diagnostic_kind").and_then(Value::as_str),
        "has_input": payload.get("input").is_some(),
        "has_output": payload.get("output").is_some(),
        "has_content": payload.get("content").is_some(),
        "has_text": payload.get("text").is_some(),
        "field_count": payload.as_object().map_or(0, serde_json::Map::len),
    })
}

#[derive(Debug)]
pub(in crate::debug) struct TraceTurnSpan {
    turn_id: String,
    started_at: Option<time::OffsetDateTime>,
    ended_at: Option<time::OffsetDateTime>,
    event_counts: BTreeMap<String, usize>,
    entry_count: usize,
    approval_count: usize,
}

impl TraceTurnSpan {
    pub(in crate::debug) fn new(turn_id: String) -> Self {
        Self {
            turn_id,
            started_at: None,
            ended_at: None,
            event_counts: BTreeMap::new(),
            entry_count: 0,
            approval_count: 0,
        }
    }

    pub(in crate::debug) fn observe_event(&mut self, event: &AgentEvent) {
        *self
            .event_counts
            .entry(trace_category(event).to_owned())
            .or_default() += 1;
        self.started_at = Some(self.started_at.map_or(event.at, |at| at.min(event.at)));
        self.ended_at = Some(self.ended_at.map_or(event.at, |at| at.max(event.at)));
    }

    pub(in crate::debug) fn into_json(self, session_id: &str) -> Value {
        let correlation_id = trace_correlation_id(session_id, &self.turn_id);
        json!({
            "turn_id": self.turn_id,
            "correlation_id": correlation_id,
            "started_at": self.started_at,
            "ended_at": self.ended_at,
            "event_counts": self.event_counts,
            "entry_count": self.entry_count,
            "approval_count": self.approval_count,
        })
    }
}

pub(in crate::debug) fn trace_event_summary(event: &AgentEvent) -> Value {
    json!({
        "event_id": event.event_id,
        "session_id": event.session_id,
        "turn_id": event.turn_id,
        "correlation_id": trace_correlation_id(&event.session_id, &event.turn_id),
        "parent_event_id": event.parent_event_id,
        "at": event.at,
        "source": event.source,
        "category": trace_category(event),
        "kind": trace_event_kind(event),
        "model_stream_kind": model_stream_event_kind(&event.kind),
        "diagnostic_kind": model_diagnostic_kind(&event.kind),
        "payload_kind": event.payload.get("kind").and_then(Value::as_str),
        "payload_phase": event.payload.get("phase").and_then(Value::as_str),
    })
}

pub(in crate::debug) fn trace_correlation_id(
    session_id: impl std::fmt::Display,
    turn_id: impl std::fmt::Display,
) -> String {
    format!("session:{session_id}:turn:{turn_id}")
}

pub(in crate::debug) fn turn_correlation_map(
    session_id: &str,
    turn_ids: &BTreeSet<String>,
) -> BTreeMap<String, String> {
    turn_ids
        .iter()
        .map(|turn_id| (turn_id.clone(), trace_correlation_id(session_id, turn_id)))
        .collect()
}

pub(in crate::debug) fn selected_turn_correlation_id(
    session_id: &str,
    requested_turn_id: Option<&str>,
    turn_ids: &BTreeSet<String>,
) -> Option<String> {
    if let Some(turn_id) = requested_turn_id {
        return Some(trace_correlation_id(session_id, turn_id));
    }
    let mut iter = turn_ids.iter();
    let only_turn = iter.next()?;
    if iter.next().is_some() {
        return None;
    }
    Some(trace_correlation_id(session_id, only_turn))
}

pub(in crate::debug) fn model_diagnostic_kind(kind: &AgentEventKind) -> Option<&str> {
    let AgentEventKind::ModelDiagnostic(diagnostic) = kind else {
        return None;
    };
    Some(diagnostic.kind.as_str())
}

pub(in crate::debug) fn trace_category(event: &AgentEvent) -> &'static str {
    match &event.kind {
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

pub(in crate::debug) fn trace_event_kind(event: &AgentEvent) -> &'static str {
    match event.kind {
        AgentEventKind::SessionStart => "session_start",
        AgentEventKind::TurnStart => "turn_start",
        AgentEventKind::UserMessage => "user_message",
        AgentEventKind::ModelStream(_) => "model_stream",
        AgentEventKind::ModelDiagnostic(_) => "model_diagnostic",
        AgentEventKind::ToolCallStarted => "tool_call_started",
        AgentEventKind::ToolCallOutputDelta => "tool_call_output_delta",
        AgentEventKind::ToolCallCompleted => "tool_call_completed",
        AgentEventKind::ToolCallFailed => "tool_call_failed",
        AgentEventKind::ToolCallCancelled => "tool_call_cancelled",
        AgentEventKind::ContextDiff => "context_diff",
        AgentEventKind::ContextCompacted => "context_compacted",
        AgentEventKind::MemoryLifecycle => "memory_lifecycle",
        AgentEventKind::CodingTurn => "coding_turn",
        AgentEventKind::AuditAnchor => "audit_anchor",
        AgentEventKind::ContinuationStarted => "continuation_started",
        AgentEventKind::ContinuationCompleted => "continuation_completed",
        AgentEventKind::ContinuationFailed => "continuation_failed",
        AgentEventKind::ContinuationCancelled => "continuation_cancelled",
        AgentEventKind::ApprovalRequested => "approval_requested",
        AgentEventKind::ApprovalResolved => "approval_resolved",
        AgentEventKind::TurnEnd => "turn_end",
        AgentEventKind::Error => "error",
    }
}

pub(in crate::debug) fn model_stream_event_kind(kind: &AgentEventKind) -> Option<&'static str> {
    let AgentEventKind::ModelStream(event) = kind else {
        return None;
    };
    Some(match event {
        ikaros_models::ModelStreamEvent::Start { .. } => "start",
        ikaros_models::ModelStreamEvent::TextDelta(_) => "text_delta",
        ikaros_models::ModelStreamEvent::ReasoningDelta(_) => "reasoning_delta",
        ikaros_models::ModelStreamEvent::ToolCallStart { .. } => "tool_call_start",
        ikaros_models::ModelStreamEvent::ToolCallDelta { .. } => "tool_call_delta",
        ikaros_models::ModelStreamEvent::ToolCallEnd { .. } => "tool_call_end",
        ikaros_models::ModelStreamEvent::RefusalDelta(_) => "refusal_delta",
        ikaros_models::ModelStreamEvent::Usage(_) => "usage",
        ikaros_models::ModelStreamEvent::Error { .. } => "error",
        ikaros_models::ModelStreamEvent::Done => "done",
    })
}
