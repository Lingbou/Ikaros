// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, anyhow};
use ikaros_core::{redact_json, redact_secrets};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSource, ContinuationId, SessionContinuation,
    SessionContinuationKind, SessionContinuationStatus, SessionId, SessionStore,
    SqliteSessionStore,
};
use serde_json::json;
use std::collections::VecDeque;

use super::{InteractiveChatRuntime, terminal_inline};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat) enum WorkbenchCancelTarget {
    All,
    Continuation(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat) struct WorkbenchCancelReport {
    pub(in crate::chat) cancelled: usize,
    pub(in crate::chat) skipped: usize,
    pub(in crate::chat) missing: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::chat) struct WorkbenchSelectedContinuationCancelReport {
    pub(in crate::chat) continuation_id: Option<String>,
    pub(in crate::chat) report: WorkbenchCancelReport,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkbenchSelectedInputClearReport {
    pub(super) input_index: Option<usize>,
    pub(super) removed: Option<String>,
    pub(super) remaining: usize,
}

pub(in crate::chat) fn cancel_session_continuations(
    store: &dyn SessionStore,
    session_id: &SessionId,
    target: WorkbenchCancelTarget,
    reason: &str,
) -> Result<WorkbenchCancelReport> {
    let continuations = store.continuations(session_id)?;
    let mut report = WorkbenchCancelReport {
        cancelled: 0,
        skipped: 0,
        missing: 0,
    };
    let mut matched = false;
    for continuation in continuations {
        let is_target = match &target {
            WorkbenchCancelTarget::All => true,
            WorkbenchCancelTarget::Continuation(target_id) => {
                continuation.continuation_id.as_str() == target_id
            }
        };
        if !is_target {
            continue;
        }
        matched = true;
        match continuation.status {
            SessionContinuationStatus::Queued | SessionContinuationStatus::Running => {
                if let Some(cancelled) =
                    store.cancel_continuation(&continuation.continuation_id, reason)?
                {
                    record_workbench_continuation_cancelled_event(store, &cancelled, reason)?;
                    report.cancelled += 1;
                } else {
                    report.missing += 1;
                }
            }
            SessionContinuationStatus::Completed
            | SessionContinuationStatus::Failed
            | SessionContinuationStatus::Cancelled => {
                report.skipped += 1;
            }
        }
    }
    if !matched && matches!(target, WorkbenchCancelTarget::Continuation(_)) {
        report.missing = 1;
    }
    Ok(report)
}

fn record_workbench_continuation_cancelled_event(
    store: &dyn SessionStore,
    continuation: &SessionContinuation,
    reason: &str,
) -> Result<()> {
    let turn_id = continuation.turn_id.clone().unwrap_or_default();
    store.append_agent_event(&AgentEvent::new(
        continuation.session_id.clone(),
        turn_id,
        None,
        AgentEventSource::Runtime,
        AgentEventKind::ContinuationCancelled,
        json!({
            "continuation_id": continuation.continuation_id.as_str(),
            "kind": continuation_kind_label(continuation.kind),
            "status": "cancelled",
            "reason": redact_secrets(reason),
            "attempt_count": continuation.attempt_count,
            "lease_owner": continuation.lease_owner.as_deref().map(redact_secrets),
            "lease_expires_at": continuation.lease_expires_at,
        }),
    ))?;
    Ok(())
}

pub(in crate::chat) fn cancel_selected_screen_continuation(
    store: &dyn SessionStore,
    session_id: &SessionId,
    approval_side_panel_rows: usize,
    pending_inputs: &VecDeque<String>,
    side_selection: usize,
    reason: &str,
) -> Result<WorkbenchSelectedContinuationCancelReport> {
    let first_continuation_row = approval_side_panel_rows + pending_inputs.len().min(4);
    let empty = || WorkbenchSelectedContinuationCancelReport {
        continuation_id: None,
        report: WorkbenchCancelReport {
            cancelled: 0,
            skipped: 0,
            missing: 0,
        },
    };
    if side_selection < first_continuation_row {
        return Ok(empty());
    }
    let continuation_index = side_selection - first_continuation_row;
    let continuations = store.continuations(session_id)?;
    let Some(continuation_id) = continuations
        .iter()
        .filter(|continuation| {
            matches!(
                continuation.status,
                SessionContinuationStatus::Queued | SessionContinuationStatus::Running
            )
        })
        .take(4)
        .nth(continuation_index)
        .map(|continuation| continuation.continuation_id.as_str().to_owned())
    else {
        return Ok(empty());
    };
    let report = cancel_session_continuations(
        store,
        session_id,
        WorkbenchCancelTarget::Continuation(continuation_id.clone()),
        reason,
    )?;
    Ok(WorkbenchSelectedContinuationCancelReport {
        continuation_id: Some(continuation_id),
        report,
    })
}

pub(super) fn clear_selected_screen_input(
    approval_side_panel_rows: usize,
    pending_inputs: &mut VecDeque<String>,
    side_selection: usize,
) -> WorkbenchSelectedInputClearReport {
    let first_input_row = approval_side_panel_rows;
    let empty = |remaining| WorkbenchSelectedInputClearReport {
        input_index: None,
        removed: None,
        remaining,
    };
    if side_selection < first_input_row {
        return empty(pending_inputs.len());
    }
    let input_index = side_selection - first_input_row;
    if input_index >= pending_inputs.len().min(4) {
        return empty(pending_inputs.len());
    }
    let removed = pending_inputs.remove(input_index);
    WorkbenchSelectedInputClearReport {
        input_index: Some(input_index + 1),
        removed,
        remaining: pending_inputs.len(),
    }
}

pub(super) fn handle_cancel_command(
    args: Vec<&str>,
    runtime: &InteractiveChatRuntime,
) -> Result<()> {
    let target = match args.as_slice() {
        [] | ["all"] => WorkbenchCancelTarget::All,
        [continuation_id] => WorkbenchCancelTarget::Continuation((*continuation_id).to_owned()),
        _ => return Err(anyhow!("usage: /cancel [all|<continuation-id>]")),
    };
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let report =
        cancel_session_continuations(&store, &session_id, target.clone(), "workbench cancel")?;
    let target_label = match target {
        WorkbenchCancelTarget::All => "all".to_owned(),
        WorkbenchCancelTarget::Continuation(id) => terminal_inline(&id),
    };
    println!(
        "workbench_cancel: target={} cancelled={} skipped={} missing={}",
        terminal_inline(&target_label),
        report.cancelled,
        report.skipped,
        report.missing
    );
    println!(
        "{}",
        continuations_json_line(&store.continuations(&session_id)?)
    );
    Ok(())
}

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

fn continuation_status_label(status: SessionContinuationStatus) -> &'static str {
    match status {
        SessionContinuationStatus::Queued => "queued",
        SessionContinuationStatus::Running => "running",
        SessionContinuationStatus::Completed => "completed",
        SessionContinuationStatus::Failed => "failed",
        SessionContinuationStatus::Cancelled => "cancelled",
    }
}

fn continuation_status_reason_label(
    reason: ikaros_session::SessionContinuationStatusReason,
) -> &'static str {
    match reason {
        ikaros_session::SessionContinuationStatusReason::Enqueued => "enqueued",
        ikaros_session::SessionContinuationStatusReason::Claimed => "claimed",
        ikaros_session::SessionContinuationStatusReason::Completed => "completed",
        ikaros_session::SessionContinuationStatusReason::Failed => "failed",
        ikaros_session::SessionContinuationStatusReason::Cancelled => "cancelled",
        ikaros_session::SessionContinuationStatusReason::Requeued => "requeued",
        ikaros_session::SessionContinuationStatusReason::LeaseExpired => "lease_expired",
    }
}

pub(super) fn print_workbench_continuation_status(runtime: &InteractiveChatRuntime) -> Result<()> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let store = SqliteSessionStore::new(&runtime.state_dir);
    let continuations = store.continuations(&session_id)?;
    let queued = continuation_status_count(&continuations, SessionContinuationStatus::Queued);
    let running = continuation_status_count(&continuations, SessionContinuationStatus::Running);
    let completed = continuation_status_count(&continuations, SessionContinuationStatus::Completed);
    let failed = continuation_status_count(&continuations, SessionContinuationStatus::Failed);
    let cancelled = continuation_status_count(&continuations, SessionContinuationStatus::Cancelled);
    println!(
        "debug_continuations: session={} total={} queued={} running={} completed={} failed={} cancelled={}",
        terminal_inline(session_id.as_str()),
        continuations.len(),
        queued,
        running,
        completed,
        failed,
        cancelled,
    );
    println!("{}", continuations_json_line(&continuations));
    Ok(())
}

pub(super) fn handle_queue_command(
    args: Vec<&str>,
    runtime: &mut InteractiveChatRuntime,
) -> Result<()> {
    match args.as_slice() {
        [] => {
            println!("pending_inputs: {}", runtime.pending_inputs.len());
            for (index, input) in runtime.pending_inputs.iter().enumerate() {
                println!("- index={} message={}", index + 1, terminal_inline(input));
            }
        }
        ["clear"] => {
            let cleared = runtime.pending_inputs.len();
            runtime.pending_inputs.clear();
            println!("pending_inputs_cleared: {cleared}");
        }
        ["run" | "drain" | "continue"] => {
            println!(
                "pending_input_run: pending_inputs={} status=handled_by_interactive_loop",
                runtime.pending_inputs.len()
            );
        }
        ["retry" | "requeue", continuation_id] => {
            let session_id = SessionId::from(runtime.chat_session_id.as_str());
            let store = SqliteSessionStore::new(&runtime.state_dir);
            let continuation_id = ContinuationId::from(*continuation_id);
            let requeued = store.requeue_continuation(
                &continuation_id,
                "workbench requeue",
                json!({
                    "requeued_by": "workbench",
                    "requeue_source": "/queue retry",
                }),
            )?;
            match requeued {
                Some(continuation) => {
                    println!(
                        "continuation_requeued: id={} session={} status={} attempts={} run=/queue run cancel=/cancel {}",
                        terminal_inline(continuation.continuation_id.as_str()),
                        terminal_inline(session_id.as_str()),
                        continuation_status_label(continuation.status),
                        continuation.attempt_count,
                        terminal_inline(continuation.continuation_id.as_str()),
                    );
                }
                None => {
                    println!(
                        "continuation_requeue_skipped: id={} reason=not_requeueable_or_missing debug=/debug continuations",
                        terminal_inline(continuation_id.as_str()),
                    );
                }
            }
            println!(
                "{}",
                continuations_json_line(&store.continuations(&session_id)?)
            );
        }
        ["retry" | "requeue"] => {
            println!("continuation_requeue_error: usage=/queue retry <continuation-id>");
        }
        ["remove", index] => match index.parse::<usize>() {
            Ok(0) | Err(_) => {
                println!("pending_input_remove_error: index must be a positive number");
            }
            Ok(index) => {
                let removed = runtime.pending_inputs.remove(index - 1);
                if let Some(removed) = removed {
                    println!(
                        "pending_input_removed: index={} remaining={} message={}",
                        index,
                        runtime.pending_inputs.len(),
                        terminal_inline(&removed)
                    );
                } else {
                    println!(
                        "pending_input_remove_error: index={} reason=not_found pending_inputs={}",
                        index,
                        runtime.pending_inputs.len()
                    );
                }
            }
        },
        _ => {
            let input = args.join(" ");
            runtime.pending_inputs.push_back(input);
            println!("pending_input_queued: {}", runtime.pending_inputs.len());
        }
    }
    println!("{}", pending_inputs_json_line(&runtime.pending_inputs));
    Ok(())
}

fn pending_inputs_json_line(pending_inputs: &VecDeque<String>) -> String {
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
