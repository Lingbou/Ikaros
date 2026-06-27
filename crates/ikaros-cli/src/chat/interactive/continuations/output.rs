// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{redact_json, redact_secrets};
use ikaros_state::session::{
    SessionContinuation, SessionContinuationKind, SessionContinuationStatus,
};
use serde_json::json;
use std::collections::VecDeque;

use super::super::terminal_inline;

pub(in crate::chat) fn continuations_json_line(continuations: &[SessionContinuation]) -> String {
    let queued = continuation_status_count(continuations, SessionContinuationStatus::Queued);
    let running = continuation_status_count(continuations, SessionContinuationStatus::Running);
    let completed = continuation_status_count(continuations, SessionContinuationStatus::Completed);
    let failed = continuation_status_count(continuations, SessionContinuationStatus::Failed);
    let cancelled = continuation_status_count(continuations, SessionContinuationStatus::Cancelled);
    let items = continuations
        .iter()
        .map(|continuation| {
            let active = matches!(
                continuation.status,
                SessionContinuationStatus::Queued | SessionContinuationStatus::Running
            );
            let retryable = matches!(
                continuation.status,
                SessionContinuationStatus::Failed | SessionContinuationStatus::Cancelled
            );
            let id = terminal_inline(continuation.continuation_id.as_str());
            let turn_id = continuation
                .turn_id
                .as_ref()
                .map(|id| terminal_inline(id.as_str()));
            json!({
                "id": id.clone(),
                "session_id": terminal_inline(continuation.session_id.as_str()),
                "turn_id": turn_id.clone(),
                "kind": continuation_kind_label(continuation.kind),
                "status": continuation_status_label(continuation.status),
                "status_reason": continuation
                    .status_reason
                    .map(continuation_status_reason_label),
                "priority": continuation.priority,
                "attempt_count": continuation.attempt_count,
                "lease_owner": continuation.lease_owner.as_deref().map(terminal_inline),
                "lease_expires_at": continuation.lease_expires_at,
                "claimed_at": continuation.claimed_at,
                "completed_at": continuation.completed_at,
                "terminal": !active,
                "active": active,
                "retryable": retryable,
                "error": continuation.error.as_deref().map(redact_secrets),
                "payload": redact_json(continuation.payload.clone()),
                "actions": {
                    "default": if active {
                        Some(format!("/cancel {id}"))
                    } else if retryable {
                        Some(format!("/queue retry {id}"))
                    } else {
                        None
                    },
                    "cancel": active.then(|| format!("/cancel {id}")),
                    "retry": retryable.then(|| format!("/queue retry {id}")),
                    "requeue": retryable.then(|| format!("/queue requeue {id}")),
                    "timeline": turn_id.as_ref().map(|turn_id| format!("/timeline {turn_id}")),
                    "trace": turn_id.as_ref().map(|turn_id| format!("/trace {turn_id}")),
                    "debug": "/debug continuations",
                },
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({
        "schema": "ikaros-workbench-continuations-v1",
        "version": 1,
        "continuation_count": continuations.len(),
        "active_count": queued + running,
        "counts": {
            "queued": queued,
            "running": running,
            "completed": completed,
            "failed": failed,
            "cancelled": cancelled,
        },
        "items": items,
        "actions": {
            "cancel_all": "/cancel all",
            "run_pending": "/queue run",
            "inspect": "/debug continuations",
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-continuations-v1","version":1,"continuation_count":0,"active_count":0,"counts":{"queued":0,"running":0,"completed":0,"failed":0,"cancelled":0},"items":[],"actions":{"cancel_all":"/cancel all"}}"#
            .to_owned()
    });
    format!("continuations_json: {encoded}")
}

pub(super) fn continuation_status_count(
    continuations: &[SessionContinuation],
    status: SessionContinuationStatus,
) -> usize {
    continuations
        .iter()
        .filter(|continuation| continuation.status == status)
        .count()
}

fn continuation_kind_label(kind: SessionContinuationKind) -> &'static str {
    match kind {
        SessionContinuationKind::Steer => "steer",
        SessionContinuationKind::FollowUp => "follow_up",
        SessionContinuationKind::NextTurn => "next_turn",
        SessionContinuationKind::Resume => "resume",
        SessionContinuationKind::Retry => "retry",
        SessionContinuationKind::Compact => "compact",
        SessionContinuationKind::ToolResult => "tool_result",
    }
}

pub(super) fn continuation_status_label(status: SessionContinuationStatus) -> &'static str {
    match status {
        SessionContinuationStatus::Queued => "queued",
        SessionContinuationStatus::Running => "running",
        SessionContinuationStatus::Completed => "completed",
        SessionContinuationStatus::Failed => "failed",
        SessionContinuationStatus::Cancelled => "cancelled",
    }
}

fn continuation_status_reason_label(
    reason: ikaros_state::session::SessionContinuationStatusReason,
) -> &'static str {
    match reason {
        ikaros_state::session::SessionContinuationStatusReason::Enqueued => "enqueued",
        ikaros_state::session::SessionContinuationStatusReason::Claimed => "claimed",
        ikaros_state::session::SessionContinuationStatusReason::Completed => "completed",
        ikaros_state::session::SessionContinuationStatusReason::Failed => "failed",
        ikaros_state::session::SessionContinuationStatusReason::Cancelled => "cancelled",
        ikaros_state::session::SessionContinuationStatusReason::Requeued => "requeued",
        ikaros_state::session::SessionContinuationStatusReason::LeaseExpired => "lease_expired",
    }
}

pub(super) fn pending_inputs_json_line(pending_inputs: &VecDeque<String>) -> String {
    let items = pending_inputs
        .iter()
        .enumerate()
        .map(|(index, input)| {
            let index = index + 1;
            json!({
                "index": index,
                "message": terminal_inline(input),
                "actions": {
                    "run": "/queue run",
                    "remove": format!("/queue remove {index}"),
                },
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({
        "schema": "ikaros-workbench-pending-inputs-v1",
        "version": 1,
        "pending_count": pending_inputs.len(),
        "status": if pending_inputs.is_empty() { "empty" } else { "queued" },
        "items": items,
        "actions": {
            "run": "/queue run",
            "clear": "/queue clear",
            "continue": "/queue continue",
            "drain": "/queue drain",
        },
        "recovery": {
            "budget": "/budget",
            "disable_budget": "/budget disable",
            "approvals": "/approval",
            "screen": "/screen --focus side",
        },
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-pending-inputs-v1","version":1,"pending_count":0,"status":"empty","items":[],"actions":{"run":"/queue run","clear":"/queue clear","continue":"/queue continue","drain":"/queue drain"},"recovery":{"budget":"/budget","disable_budget":"/budget disable","approvals":"/approval","screen":"/screen --focus side"}}"#
            .to_owned()
    });
    format!("pending_inputs_json: {encoded}")
}
