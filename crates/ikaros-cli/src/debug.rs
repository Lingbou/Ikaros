// SPDX-License-Identifier: GPL-3.0-only

use crate::resolve_agent_instance;
use anyhow::{Result, anyhow};
use clap::{Args, Subcommand};
use ikaros_core::{IkarosConfig, IkarosPaths, redact_json};
use ikaros_memory::{JsonlMemoryJournal, MemoryJournal, MemoryRef};
use ikaros_session::{
    AgentEvent, AgentEventKind, SessionContinuation, SessionContinuationStatus,
    SessionContinuationStatusReason, SessionId, SessionReplay, SessionStore, SqliteSessionStore,
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Subcommand)]
pub(crate) enum DebugCommand {
    ContextDiff(DebugSessionQuery),
    MemoryLifecycle(DebugSessionQuery),
    Continuations(DebugSessionQuery),
    CodingTurn(DebugSessionQuery),
    Trace(DebugSessionQuery),
}

#[derive(Debug, Args)]
pub(crate) struct DebugSessionQuery {
    session_id: String,
    #[arg(long)]
    turn_id: Option<String>,
}

pub(crate) fn debug_command(
    command: DebugCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        DebugCommand::ContextDiff(args) => {
            debug_context_diff(args, paths, workspace, agent_override)
        }
        DebugCommand::MemoryLifecycle(args) => {
            debug_memory_lifecycle(args, paths, workspace, agent_override)
        }
        DebugCommand::Continuations(args) => {
            debug_continuations(args, paths, workspace, agent_override)
        }
        DebugCommand::CodingTurn(args) => debug_coding_turn(args, paths, workspace, agent_override),
        DebugCommand::Trace(args) => debug_trace(args, paths, workspace, agent_override),
    }
}

fn debug_trace(
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
        .map(TraceTurnSpan::into_json)
        .collect::<Vec<_>>();
    let ordered_events = events
        .into_iter()
        .map(trace_event_summary)
        .collect::<Vec<_>>();
    let output = json!({
        "format": "ikaros-trace-v1",
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "state_db": state_db.display().to_string(),
        "session_source": replay.session.source,
        "event_counts": event_counts,
        "turn_spans": turn_spans,
        "ordered_events": ordered_events,
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

#[derive(Debug)]
struct TraceTurnSpan {
    turn_id: String,
    started_at: Option<time::OffsetDateTime>,
    ended_at: Option<time::OffsetDateTime>,
    event_counts: BTreeMap<String, usize>,
    entry_count: usize,
    approval_count: usize,
}

impl TraceTurnSpan {
    fn new(turn_id: String) -> Self {
        Self {
            turn_id,
            started_at: None,
            ended_at: None,
            event_counts: BTreeMap::new(),
            entry_count: 0,
            approval_count: 0,
        }
    }

    fn observe_event(&mut self, event: &AgentEvent) {
        *self
            .event_counts
            .entry(trace_category(event).to_owned())
            .or_default() += 1;
        self.started_at = Some(self.started_at.map_or(event.at, |at| at.min(event.at)));
        self.ended_at = Some(self.ended_at.map_or(event.at, |at| at.max(event.at)));
    }

    fn into_json(self) -> Value {
        json!({
            "turn_id": self.turn_id,
            "started_at": self.started_at,
            "ended_at": self.ended_at,
            "event_counts": self.event_counts,
            "entry_count": self.entry_count,
            "approval_count": self.approval_count,
        })
    }
}

fn trace_event_summary(event: &AgentEvent) -> Value {
    json!({
        "event_id": event.event_id,
        "session_id": event.session_id,
        "turn_id": event.turn_id,
        "parent_event_id": event.parent_event_id,
        "at": event.at,
        "source": event.source,
        "category": trace_category(event),
        "kind": trace_event_kind(event),
        "model_stream_kind": model_stream_event_kind(&event.kind),
        "payload_kind": event.payload.get("kind").and_then(Value::as_str),
        "payload_phase": event.payload.get("phase").and_then(Value::as_str),
    })
}

fn trace_category(event: &AgentEvent) -> &'static str {
    match &event.kind {
        AgentEventKind::ModelStream(_) => "model",
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

fn trace_event_kind(event: &AgentEvent) -> &'static str {
    match event.kind {
        AgentEventKind::SessionStart => "session_start",
        AgentEventKind::TurnStart => "turn_start",
        AgentEventKind::UserMessage => "user_message",
        AgentEventKind::ModelStream(_) => "model_stream",
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

fn model_stream_event_kind(kind: &AgentEventKind) -> Option<&'static str> {
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

fn debug_coding_turn(
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
    let coding_events = events
        .into_iter()
        .filter(|event| matches!(event.kind, AgentEventKind::CodingTurn))
        .collect::<Vec<_>>();
    let mut event_kind_counts = BTreeMap::<String, usize>::new();
    for event in &coding_events {
        if let Some(kind) = event.payload["kind"].as_str() {
            *event_kind_counts.entry(kind.to_owned()).or_default() += 1;
        }
    }
    let entries = replay
        .entries
        .iter()
        .filter(|entry| {
            args.turn_id.as_deref().is_none_or(|turn_id| {
                entry
                    .turn_id
                    .as_ref()
                    .is_some_and(|entry_turn_id| entry_turn_id.as_str() == turn_id)
            }) && entry.payload["kind"].as_str().is_some()
        })
        .map(|entry| {
            json!({
                "entry_id": entry.entry_id.clone(),
                "turn_id": entry.turn_id.clone(),
                "kind": entry.kind,
                "coding_kind": entry.payload["kind"].as_str(),
                "visible_text": entry.visible_text.clone(),
                "payload": entry.payload.clone(),
            })
        })
        .collect::<Vec<_>>();
    let review_findings = coding_events
        .iter()
        .filter(|event| event.payload["kind"].as_str() == Some("review_finding"))
        .map(|event| event.payload.clone())
        .collect::<Vec<_>>();
    let output = json!({
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "state_db": state_db.display().to_string(),
        "event_count": coding_events.len(),
        "entry_count": entries.len(),
        "event_kind_counts": event_kind_counts,
        "review_findings": review_findings,
        "events": coding_events,
        "entries": entries,
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

fn debug_context_diff(
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
    let context_events = events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::ContextDiff))
        .collect::<Vec<_>>();
    let compacted_events = events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::ContextCompacted))
        .collect::<Vec<_>>();
    let context_errors = events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::Error))
        .filter(|event| {
            event.payload["phase"].as_str() == Some("context_assemble")
                || event.payload["message"]
                    .as_str()
                    .is_some_and(|message| message.contains("context limit exceeded"))
        })
        .collect::<Vec<_>>();
    let turn_ids = context_events
        .iter()
        .chain(compacted_events.iter())
        .chain(context_errors.iter())
        .map(|event| event.turn_id.to_string())
        .collect::<BTreeSet<_>>();
    let latest_context = context_events.last().map(|event| &event.payload);
    let latest_compaction = compacted_events.last().map(|event| &event.payload);

    let output = json!({
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "state_db": state_db.display().to_string(),
        "turns": turn_ids,
        "context_diff_events": context_events.len(),
        "context_compacted": !compacted_events.is_empty(),
        "context_limit_error": context_errors.last().map(|event| &event.payload),
        "budget": latest_context.and_then(|payload| payload.get("budget")).cloned(),
        "estimator": latest_context
            .and_then(|payload| payload.pointer("/budget/estimator"))
            .and_then(Value::as_str),
        "context_window": latest_context
            .and_then(|payload| payload.pointer("/budget/context_window"))
            .and_then(Value::as_u64),
        "sections": latest_context.and_then(|payload| payload.get("sections")).cloned(),
        "diff": latest_context.and_then(|payload| payload.get("diff")).cloned(),
        "references": latest_context.and_then(|payload| payload.get("references")).cloned(),
        "compressed_sections": latest_context
            .and_then(|payload| payload.get("compressed_sections"))
            .cloned(),
        "protected_sections": latest_context
            .and_then(|payload| payload.get("protected_sections"))
            .cloned(),
        "compression_summary": latest_context
            .and_then(|payload| payload.get("compression_summary"))
            .cloned(),
        "continuation_prompt": latest_context
            .and_then(|payload| payload.get("continuation_prompt"))
            .cloned(),
        "compaction_event": latest_compaction.cloned(),
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

fn debug_memory_lifecycle(
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
    let memory_events = events
        .into_iter()
        .filter(|event| matches!(event.kind, AgentEventKind::MemoryLifecycle))
        .map(memory_event_summary)
        .collect::<Vec<_>>();
    let journal = JsonlMemoryJournal::new(&paths.memory_dir);
    let matching_journal_entries = journal
        .list()?
        .into_iter()
        .filter(|entry| {
            source_ref_matches(
                entry.source_ref.as_ref(),
                &args.session_id,
                args.turn_id.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    let mut action_counts = BTreeMap::<String, usize>::new();
    for entry in &matching_journal_entries {
        *action_counts
            .entry(memory_journal_action_name(&entry.action).to_owned())
            .or_default() += 1;
    }
    let journal_entries = matching_journal_entries
        .into_iter()
        .map(serde_json::to_value)
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let output = json!({
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "state_db": state_db.display().to_string(),
        "memory_lifecycle_events": memory_events,
        "memory_journal_path": journal.path().display().to_string(),
        "memory_journal_action_counts": action_counts,
        "memory_journal_entries": journal_entries,
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

fn debug_continuations(
    args: DebugSessionQuery,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (state_db, replay) = replay_session(paths, workspace, agent_override, &args.session_id)?;
    let store = SqliteSessionStore::from_file(&state_db);
    let all_continuations = store.continuations(&replay.session.session_id)?;
    let continuations = all_continuations
        .iter()
        .filter(|continuation| {
            args.turn_id.as_deref().is_none_or(|turn_id| {
                continuation.turn_id.as_ref().map(|id| id.as_str()) == Some(turn_id)
            })
        })
        .collect::<Vec<_>>();
    if let Some(turn_id) = args.turn_id.as_deref()
        && continuations.is_empty()
        && !replay_contains_turn(&replay, turn_id)
    {
        return Err(anyhow!(
            "turn not found in session {}: {turn_id}",
            args.session_id
        ));
    }

    let mut status_counts = BTreeMap::<String, usize>::new();
    for continuation in &continuations {
        *status_counts
            .entry(continuation_status_name(continuation.status).to_owned())
            .or_default() += 1;
    }
    let now = time::OffsetDateTime::now_utc();
    let continuation_summaries = continuations
        .into_iter()
        .map(|continuation| continuation_debug_summary(continuation, now))
        .collect::<Vec<_>>();
    let output = json!({
        "session_id": args.session_id,
        "turn_id": args.turn_id,
        "state_db": state_db.display().to_string(),
        "continuation_count": continuation_summaries.len(),
        "status_counts": status_counts,
        "continuations": continuation_summaries,
    });
    println!("{}", serde_json::to_string_pretty(&redact_json(output))?);
    Ok(())
}

fn memory_journal_action_name(action: &ikaros_memory::MemoryJournalAction) -> &'static str {
    match action {
        ikaros_memory::MemoryJournalAction::Append => "append",
        ikaros_memory::MemoryJournalAction::Update => "update",
        ikaros_memory::MemoryJournalAction::Promote => "promote",
        ikaros_memory::MemoryJournalAction::Demote => "demote",
        ikaros_memory::MemoryJournalAction::Forget => "forget",
        ikaros_memory::MemoryJournalAction::Skip => "skip",
        ikaros_memory::MemoryJournalAction::CandidateAccepted => "candidate_accepted",
        ikaros_memory::MemoryJournalAction::CandidateRejected => "candidate_rejected",
        ikaros_memory::MemoryJournalAction::ProjectionRendered => "projection_rendered",
        ikaros_memory::MemoryJournalAction::Superseded => "superseded",
        ikaros_memory::MemoryJournalAction::WorkingMemoryExpired => "working_memory_expired",
    }
}

fn continuation_debug_summary(
    continuation: &SessionContinuation,
    now: time::OffsetDateTime,
) -> Value {
    json!({
        "continuation_id": continuation.continuation_id,
        "session_id": continuation.session_id,
        "turn_id": continuation.turn_id,
        "parent_continuation_id": continuation.parent_continuation_id,
        "kind": continuation.kind,
        "status": continuation.status,
        "status_reason": continuation.status_reason,
        "priority": continuation.priority,
        "attempt_count": continuation.attempt_count,
        "created_at": continuation.created_at,
        "updated_at": continuation.updated_at,
        "claimed_at": continuation.claimed_at,
        "completed_at": continuation.completed_at,
        "lease_owner": continuation.lease_owner,
        "lease_expires_at": continuation.lease_expires_at,
        "lease_expired": continuation.lease_expires_at.is_some_and(|expires_at| {
            continuation.status == SessionContinuationStatus::Running && expires_at <= now
        }),
        "terminal": continuation_terminal_summary(continuation, now),
        "error": continuation.error,
        "payload": continuation.payload,
    })
}

fn continuation_terminal_summary(
    continuation: &SessionContinuation,
    now: time::OffsetDateTime,
) -> Value {
    let lease_expired = continuation.lease_expires_at.is_some_and(|expires_at| {
        continuation.status == SessionContinuationStatus::Running && expires_at <= now
    }) || continuation.status_reason
        == Some(SessionContinuationStatusReason::LeaseExpired);
    let reason = continuation
        .status_reason
        .map(continuation_status_reason_name)
        .unwrap_or_else(|| continuation_status_name(continuation.status));
    let timeout = if lease_expired {
        json!({
            "kind": "worker_lease",
            "reason": "worker_lease_expired",
            "started_at": continuation.claimed_at,
            "ended_at": continuation.completed_at.unwrap_or(continuation.updated_at),
            "lease_owner": continuation.lease_owner.as_deref(),
            "attempt_count": continuation.attempt_count,
        })
    } else {
        Value::Null
    };
    json!({
        "reason": reason,
        "message": continuation.error.as_deref(),
        "started_at": continuation.claimed_at,
        "ended_at": continuation.completed_at.unwrap_or(continuation.updated_at),
        "lease_owner": continuation.lease_owner.as_deref(),
        "attempt_count": continuation.attempt_count,
        "timeout": timeout,
    })
}

fn continuation_status_name(status: SessionContinuationStatus) -> &'static str {
    match status {
        SessionContinuationStatus::Queued => "queued",
        SessionContinuationStatus::Running => "running",
        SessionContinuationStatus::Completed => "completed",
        SessionContinuationStatus::Failed => "failed",
        SessionContinuationStatus::Cancelled => "cancelled",
    }
}

fn continuation_status_reason_name(reason: SessionContinuationStatusReason) -> &'static str {
    match reason {
        SessionContinuationStatusReason::Enqueued => "enqueued",
        SessionContinuationStatusReason::Claimed => "claimed",
        SessionContinuationStatusReason::Completed => "completed",
        SessionContinuationStatusReason::Failed => "failed",
        SessionContinuationStatusReason::Cancelled => "cancelled",
        SessionContinuationStatusReason::Requeued => "requeued",
        SessionContinuationStatusReason::LeaseExpired => "lease_expired",
    }
}

fn replay_contains_turn(replay: &SessionReplay, turn_id: &str) -> bool {
    replay
        .agent_events
        .iter()
        .any(|event| event.turn_id.as_str() == turn_id)
        || replay.entries.iter().any(|entry| {
            entry
                .turn_id
                .as_ref()
                .is_some_and(|entry_turn_id| entry_turn_id.as_str() == turn_id)
        })
        || replay.approvals.iter().any(|approval| {
            approval
                .turn_id
                .as_ref()
                .is_some_and(|approval_turn_id| approval_turn_id.as_str() == turn_id)
        })
}

fn replay_session(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: &str,
) -> Result<(PathBuf, SessionReplay)> {
    let session_id = SessionId::from(session_id);
    for state_db in state_db_candidates(paths, workspace, agent_override)? {
        let store = SqliteSessionStore::from_file(&state_db);
        if let Some(replay) = store.replay_session(&session_id)? {
            return Ok((state_db, replay));
        }
    }
    Err(anyhow!("session not found in state.db files: {session_id}"))
}

fn state_db_candidates(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Vec<PathBuf>> {
    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    let config = IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    push_state_db_candidate(&mut candidates, &mut seen, agent.state_dir.join("state.db"));
    if agent_override.is_none() {
        let agents_dir = paths.home.join("agents");
        if agents_dir.is_dir() {
            for entry in fs::read_dir(&agents_dir)? {
                let entry = entry?;
                let state_db = entry.path().join("state.db");
                push_state_db_candidate(&mut candidates, &mut seen, state_db);
            }
        }
    }
    Ok(candidates)
}

fn push_state_db_candidate(
    candidates: &mut Vec<PathBuf>,
    seen: &mut BTreeSet<PathBuf>,
    state_db: PathBuf,
) {
    if state_db.is_file() && seen.insert(state_db.clone()) {
        candidates.push(state_db);
    }
}

fn filter_turn_events<'a>(
    events: &'a [AgentEvent],
    session_id: &str,
    turn_id: Option<&str>,
) -> Result<Vec<&'a AgentEvent>> {
    let filtered = events
        .iter()
        .filter(|event| turn_id.is_none_or(|turn_id| event.turn_id.as_str() == turn_id))
        .collect::<Vec<_>>();
    if let Some(turn_id) = turn_id
        && filtered.is_empty()
    {
        return Err(anyhow!("turn not found in session {session_id}: {turn_id}"));
    }
    Ok(filtered)
}

fn memory_event_summary(event: &AgentEvent) -> Value {
    let notes = event.payload["report"]["notes"]
        .as_array()
        .or_else(|| event.payload["notes"].as_array())
        .cloned()
        .unwrap_or_default();
    let skipped = notes.iter().any(|note| {
        note.as_str()
            .is_some_and(|note| note.to_ascii_lowercase().contains("skipped"))
    });
    let redaction_related = notes.iter().any(|note| {
        note.as_str().is_some_and(|note| {
            let note = note.to_ascii_lowercase();
            note.contains("redacted") || note.contains("secret")
        })
    });
    json!({
        "event_id": event.event_id,
        "turn_id": event.turn_id,
        "phase": event.payload["phase"].as_str()
            .or_else(|| event.payload.pointer("/report/phase").and_then(Value::as_str)),
        "records_read": event.payload["records_read"].as_u64()
            .or_else(|| event.payload.pointer("/report/records_read").and_then(Value::as_u64)),
        "records_written": event.payload["records_written"].as_u64()
            .or_else(|| event.payload.pointer("/report/records_written").and_then(Value::as_u64)),
        "source_ref": event.payload.get("source_ref")
            .cloned()
            .or_else(|| event.payload.pointer("/report/source_ref").cloned()),
        "notes": notes,
        "skipped": skipped,
        "redaction_related": redaction_related,
        "payload": event.payload,
    })
}

fn source_ref_matches(
    source_ref: Option<&MemoryRef>,
    session_id: &str,
    turn_id: Option<&str>,
) -> bool {
    match source_ref {
        Some(MemoryRef::SessionTurn {
            session_id: source_session_id,
            turn_id: source_turn_id,
        }) => {
            source_session_id == session_id
                && turn_id.is_none_or(|turn_id| source_turn_id.as_deref() == Some(turn_id))
        }
        _ => false,
    }
}
