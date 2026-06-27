// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::debug) fn debug_continuations(
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
pub(in crate::debug) fn continuation_debug_summary(
    continuation: &SessionContinuation,
    now: time::OffsetDateTime,
) -> Value {
    let correlation_id = continuation
        .turn_id
        .as_ref()
        .map(|turn_id| trace_correlation_id(&continuation.session_id, turn_id));
    json!({
        "continuation_id": continuation.continuation_id,
        "session_id": continuation.session_id,
        "turn_id": continuation.turn_id,
        "correlation_id": correlation_id,
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

pub(in crate::debug) fn continuation_terminal_summary(
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

pub(in crate::debug) fn continuation_status_name(
    status: SessionContinuationStatus,
) -> &'static str {
    match status {
        SessionContinuationStatus::Queued => "queued",
        SessionContinuationStatus::Running => "running",
        SessionContinuationStatus::Completed => "completed",
        SessionContinuationStatus::Failed => "failed",
        SessionContinuationStatus::Cancelled => "cancelled",
    }
}

pub(in crate::debug) fn continuation_status_reason_name(
    reason: SessionContinuationStatusReason,
) -> &'static str {
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

pub(in crate::debug) fn replay_contains_turn(replay: &SessionReplay, turn_id: &str) -> bool {
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
