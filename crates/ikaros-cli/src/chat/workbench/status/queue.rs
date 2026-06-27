// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::attachments::{content_block_kind, content_block_summary};
use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{IkarosConfig, IkarosPaths, redact_secrets};
use ikaros_harness::ApprovalRecord;
use ikaros_models::ModelContentBlock;
use ikaros_session::{
    SessionContinuation, SessionContinuationKind, SessionContinuationStatus, SessionId,
    SessionStore, SqliteSessionStore,
};
use std::{collections::VecDeque, path::Path};

use super::super::{WorkbenchCell, WorkbenchCellKind, terminal_inline};
use super::approval::screen_approval_cells;
use super::{state_db_candidates, truncate_chars};

pub(super) fn continuation_count(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<usize> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    let mut count = 0;
    for state_db in state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if store.get_session(&session_id)?.is_some() {
            count += store.continuations(&session_id)?.len();
        }
    }
    Ok(count)
}

pub(super) fn screen_queue_status_cell(continuations: &[SessionContinuation]) -> WorkbenchCell {
    let queued = continuation_status_count(continuations, SessionContinuationStatus::Queued);
    let running = continuation_status_count(continuations, SessionContinuationStatus::Running);
    let completed = continuation_status_count(continuations, SessionContinuationStatus::Completed);
    let failed = continuation_status_count(continuations, SessionContinuationStatus::Failed);
    let cancelled = continuation_status_count(continuations, SessionContinuationStatus::Cancelled);
    let active = continuations
        .iter()
        .find(|continuation| continuation.status == SessionContinuationStatus::Running)
        .or_else(|| {
            continuations
                .iter()
                .find(|continuation| continuation.status == SessionContinuationStatus::Queued)
        })
        .or_else(|| {
            continuations
                .iter()
                .rev()
                .find(|continuation| continuation.status == SessionContinuationStatus::Failed)
        })
        .or_else(|| {
            continuations
                .iter()
                .rev()
                .find(|continuation| continuation.status == SessionContinuationStatus::Cancelled)
        });

    let (kind, id, turn, lease_owner, attempts, error) = active
        .map(|continuation| {
            (
                continuation_kind_name(continuation.kind),
                terminal_inline(continuation.continuation_id.as_str()),
                continuation
                    .turn_id
                    .as_ref()
                    .map(|turn_id| terminal_inline(turn_id.as_str()))
                    .unwrap_or_else(|| "none".into()),
                continuation
                    .lease_owner
                    .as_deref()
                    .map(terminal_inline)
                    .unwrap_or_else(|| "none".into()),
                continuation.attempt_count.to_string(),
                continuation
                    .error
                    .as_deref()
                    .map(redact_secrets)
                    .map(|value| truncate_chars(&terminal_inline(&value), 120))
                    .unwrap_or_else(|| "none".into()),
            )
        })
        .unwrap_or_else(|| {
            (
                "none",
                "none".into(),
                "none".into(),
                "none".into(),
                "0".into(),
                "none".into(),
            )
        });

    WorkbenchCell {
        kind: WorkbenchCellKind::Continuation,
        title: "queue".into(),
        detail: format!(
            "queued={} running={} completed={} failed={} cancelled={} active_kind={} active_id={} active_turn={} lease_owner={} attempts={} error={} command=/debug continuations continue_hint=/queue run timeline=/timeline cancel_hint=/cancel",
            queued,
            running,
            completed,
            failed,
            cancelled,
            kind,
            id,
            turn,
            lease_owner,
            attempts,
            error,
        ),
    }
}

fn continuation_status_count(
    continuations: &[SessionContinuation],
    status: SessionContinuationStatus,
) -> usize {
    continuations
        .iter()
        .filter(|continuation| continuation.status == status)
        .count()
}

pub(super) fn screen_side_cells(
    pending: &[ApprovalRecord],
    continuations: &[SessionContinuation],
    pending_inputs: &VecDeque<String>,
    pending_content_blocks: &[ModelContentBlock],
) -> Vec<WorkbenchCell> {
    let mut cells = if pending.is_empty() {
        Vec::new()
    } else {
        screen_approval_cells(pending)
    };
    let mut input_queue_cells = screen_pending_input_cells(pending_inputs);
    let mut attachment_cells = screen_pending_attachment_cells(pending_content_blocks);
    let mut continuation_cells = screen_continuation_cells(continuations);
    let mut control_cells = screen_queue_control_cells(
        pending,
        continuations,
        pending_inputs,
        pending_content_blocks,
    );
    if cells.is_empty()
        && input_queue_cells.is_empty()
        && attachment_cells.is_empty()
        && continuation_cells.is_empty()
        && control_cells.is_empty()
    {
        cells.push(WorkbenchCell {
            kind: WorkbenchCellKind::Approval,
            title: "approvals".into(),
            detail: "none pending".into(),
        });
    }
    cells.append(&mut control_cells);
    cells.append(&mut input_queue_cells);
    cells.append(&mut attachment_cells);
    cells.append(&mut continuation_cells);
    cells
}

fn screen_queue_control_cells(
    pending: &[ApprovalRecord],
    continuations: &[SessionContinuation],
    pending_inputs: &VecDeque<String>,
    pending_content_blocks: &[ModelContentBlock],
) -> Vec<WorkbenchCell> {
    let queued = continuation_status_count(continuations, SessionContinuationStatus::Queued);
    let running = continuation_status_count(continuations, SessionContinuationStatus::Running);
    let failed = continuation_status_count(continuations, SessionContinuationStatus::Failed);
    let cancelled = continuation_status_count(continuations, SessionContinuationStatus::Cancelled);
    let has_work = !pending.is_empty()
        || !continuations.is_empty()
        || !pending_inputs.is_empty()
        || !pending_content_blocks.is_empty();
    if !has_work {
        return Vec::new();
    }
    let mut cells = vec![WorkbenchCell {
        kind: WorkbenchCellKind::Continuation,
        title: "queue controls".into(),
        detail: format!(
            "approvals={} pending_inputs={} attachments={} queued={} running={} failed={} cancelled={} run=/queue run cancel=/cancel all approval=/approval debug=/debug continuations",
            pending.len(),
            pending_inputs.len(),
            pending_content_blocks.len(),
            queued,
            running,
            failed,
            cancelled,
        ),
    }];
    if failed > 0 {
        cells.push(WorkbenchCell {
            kind: WorkbenchCellKind::Error,
            title: "queue recovery".into(),
            detail: format!(
                "failed={} inspect=/debug continuations timeline=/timeline --failed trace=/trace --failed run=/queue run cancel=/cancel all",
                failed
            ),
        });
    }
    if running > 0 {
        cells.push(WorkbenchCell {
            kind: WorkbenchCellKind::Continuation,
            title: "interrupt running work".into(),
            detail: format!(
                "running={} cancel=/cancel all inspect=/debug continuations trace=/trace --kind continuation",
                running
            ),
        });
    }
    cells
}

fn screen_pending_input_cells(pending_inputs: &VecDeque<String>) -> Vec<WorkbenchCell> {
    pending_inputs
        .iter()
        .take(4)
        .enumerate()
        .map(|(index, input)| WorkbenchCell {
            kind: WorkbenchCellKind::Continuation,
            title: format!("input queue {}", index + 1),
            detail: format!(
                "pending_inputs={} index={} message={} command=/queue continue_hint=/queue run clear=/queue remove {} clear_all=/queue clear",
                pending_inputs.len(),
                index + 1,
                terminal_inline(input),
                index + 1,
            ),
        })
        .collect()
}

fn screen_pending_attachment_cells(
    pending_content_blocks: &[ModelContentBlock],
) -> Vec<WorkbenchCell> {
    pending_content_blocks
        .iter()
        .take(4)
        .enumerate()
        .map(|(index, block)| WorkbenchCell {
            kind: WorkbenchCellKind::Context,
            title: format!("attachment queue {}", index + 1),
            detail: format!(
                "pending_attachments={} index={} kind={} summary={} clear=/attach remove {} clear_all=/attach clear command=/attach list",
                pending_content_blocks.len(),
                index + 1,
                content_block_kind(block),
                terminal_inline(&content_block_summary(block)),
                index + 1,
            ),
        })
        .collect()
}

fn screen_continuation_cells(continuations: &[SessionContinuation]) -> Vec<WorkbenchCell> {
    continuations
        .iter()
        .filter(|continuation| {
            matches!(
                continuation.status,
                SessionContinuationStatus::Queued
                    | SessionContinuationStatus::Running
                    | SessionContinuationStatus::Failed
                    | SessionContinuationStatus::Cancelled
            )
        })
        .take(4)
        .map(|continuation| {
            let continuation_id = terminal_inline(continuation.continuation_id.as_str());
            let cancel = if matches!(
                continuation.status,
                SessionContinuationStatus::Queued | SessionContinuationStatus::Running
            ) {
                format!(" cancel=/cancel {continuation_id}")
            } else {
                String::new()
            };
            let retry = if matches!(
                continuation.status,
                SessionContinuationStatus::Failed | SessionContinuationStatus::Cancelled
            ) {
                format!(" retry=/queue retry {continuation_id}")
            } else {
                String::new()
            };
            let turn = continuation
                .turn_id
                .as_ref()
                .map(|turn_id| terminal_inline(turn_id.as_str()))
                .unwrap_or_else(|| "none".into());
            WorkbenchCell {
                kind: if continuation.status == SessionContinuationStatus::Failed {
                    WorkbenchCellKind::Error
                } else {
                    WorkbenchCellKind::Continuation
                },
                title: format!("queue {}", continuation_kind_name(continuation.kind)),
                detail: format!(
                    "id={} status={} reason={} turn={} lease_owner={} attempts={} error={}{}{} debug=/debug continuations timeline=/timeline {} trace=/trace {}",
                    continuation_id,
                    continuation_status_name(continuation.status),
                    continuation
                        .status_reason
                        .as_ref()
                        .map(|reason| format!("{:?}", reason))
                        .unwrap_or_else(|| "none".into()),
                    turn,
                    continuation
                        .lease_owner
                        .as_deref()
                        .map(terminal_inline)
                        .unwrap_or_else(|| "none".into()),
                    continuation.attempt_count,
                    continuation
                        .error
                        .as_deref()
                        .map(redact_secrets)
                        .map(|value| truncate_chars(&terminal_inline(&value), 120))
                        .unwrap_or_else(|| "none".into()),
                    cancel,
                    retry,
                    continuation
                        .turn_id
                        .as_ref()
                        .map(|turn_id| terminal_inline(turn_id.as_str()))
                        .unwrap_or_else(|| "--kind continuation".into()),
                    continuation
                        .turn_id
                        .as_ref()
                        .map(|turn_id| terminal_inline(turn_id.as_str()))
                        .unwrap_or_else(|| "--kind continuation".into()),
                ),
            }
        })
        .collect()
}

pub(super) fn screen_continuations(
    config: &IkarosConfig,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
) -> Result<Vec<SessionContinuation>> {
    let session_id = SessionId::from(runtime.chat_session_id.as_str());
    for state_db in state_db_candidates(config, paths, workspace, runtime)? {
        if !state_db.exists() {
            continue;
        }
        let store = SqliteSessionStore::from_file(state_db);
        if store.replay_session(&session_id)?.is_some() {
            return Ok(store.continuations(&session_id)?);
        }
    }
    Ok(Vec::new())
}

fn continuation_kind_name(kind: SessionContinuationKind) -> &'static str {
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

fn continuation_status_name(status: SessionContinuationStatus) -> &'static str {
    match status {
        SessionContinuationStatus::Queued => "queued",
        SessionContinuationStatus::Running => "running",
        SessionContinuationStatus::Completed => "completed",
        SessionContinuationStatus::Failed => "failed",
        SessionContinuationStatus::Cancelled => "cancelled",
    }
}
