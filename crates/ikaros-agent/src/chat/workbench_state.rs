// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_core::redact_secrets;
use ikaros_state::session::{
    AgentEvent, AgentEventKind, AgentEventSource, ContinuationId, SessionBranchSummaryInput,
    SessionContinuation, SessionContinuationKind, SessionContinuationStatus, SessionEntry,
    SessionEntryId, SessionEntryKind, SessionId, SessionStore,
};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchCancelTarget {
    All,
    Continuation(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkbenchCancelReport {
    pub cancelled: usize,
    pub skipped: usize,
    pub missing: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkbenchForkReport {
    Forked {
        session_id: SessionId,
        parent_entry_id: SessionEntryId,
        entry: Box<SessionEntry>,
        summary: String,
    },
    SessionNotFound {
        session_id: SessionId,
    },
    MissingActiveLeaf {
        session_id: SessionId,
    },
}

pub fn append_workbench_evidence_entry(
    store: &dyn SessionStore,
    session_id: &SessionId,
    agent_id: &str,
    workspace: &Path,
    kind: &str,
    visible_text: impl Into<String>,
    payload: serde_json::Value,
) -> Result<SessionEntry> {
    let parent_entry_id = store
        .get_session(session_id)?
        .and_then(|session| session.active_leaf_entry_id);
    let mut entry = SessionEntry::new(session_id.clone(), SessionEntryKind::Custom);
    entry.parent_entry_id = parent_entry_id;
    entry.visible_text = Some(visible_text.into());
    entry.payload = json!({
        "operation": "workbench_evidence",
        "kind": kind,
        "session_id": session_id.as_str(),
        "agent_id": agent_id,
        "workspace": workspace.display().to_string(),
        "data": payload,
    });
    store.append_entry(&entry)?;
    Ok(entry)
}

pub fn fork_workbench_session(
    store: &dyn SessionStore,
    session_id: &SessionId,
    agent_id: &str,
    workspace: &Path,
    summary: impl Into<String>,
) -> Result<WorkbenchForkReport> {
    let summary = summary.into();
    let Some(session) = store.get_session(session_id)? else {
        return Ok(WorkbenchForkReport::SessionNotFound {
            session_id: session_id.clone(),
        });
    };
    let Some(parent_entry_id) = session.active_leaf_entry_id else {
        return Ok(WorkbenchForkReport::MissingActiveLeaf {
            session_id: session_id.clone(),
        });
    };
    let entry = store.branch_from_entry(&SessionBranchSummaryInput {
        session_id: session_id.clone(),
        parent_entry_id: parent_entry_id.clone(),
        summary: summary.clone(),
        payload: json!({
            "source": "workbench",
            "command": "/fork",
            "agent_id": agent_id,
            "workspace": workspace.display().to_string(),
        }),
    })?;
    Ok(WorkbenchForkReport::Forked {
        session_id: session_id.clone(),
        parent_entry_id,
        entry: Box::new(entry),
        summary,
    })
}

pub fn requeue_workbench_continuation(
    store: &dyn SessionStore,
    continuation_id: &ContinuationId,
) -> Result<Option<SessionContinuation>> {
    Ok(store.requeue_continuation(
        continuation_id,
        "workbench requeue",
        json!({
            "requeued_by": "workbench",
            "requeue_source": "/queue retry",
        }),
    )?)
}

pub fn cancel_session_continuations(
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

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_state::session::{
        AgentEventKind, SessionContinuationClaim, SessionContinuationInput,
        SessionContinuationKind, SessionContinuationStatus, SessionEntryKind, SessionRecord,
        SessionSource, SqliteSessionStore,
    };
    use serde_json::json;

    #[test]
    fn workbench_cancel_marks_queued_and_running_continuations_cancelled() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("cancel-session");
        let queued = store
            .enqueue_continuation(&SessionContinuationInput::new(
                session_id.clone(),
                SessionContinuationKind::NextTurn,
            ))
            .expect("queued continuation");
        let running = store
            .enqueue_continuation(&SessionContinuationInput::new(
                session_id.clone(),
                SessionContinuationKind::ToolResult,
            ))
            .expect("running continuation");
        let claimed = store
            .claim_next_continuation(
                &SessionContinuationClaim::for_session(session_id.clone())
                    .with_lease_owner("test-worker")
                    .with_lease_duration_seconds(30),
            )
            .expect("claim")
            .expect("claimed continuation");
        assert_eq!(claimed.continuation_id, queued.continuation_id);

        let report = cancel_session_continuations(
            &store,
            &session_id,
            WorkbenchCancelTarget::All,
            "operator requested cancel",
        )
        .expect("cancel continuations");

        assert_eq!(report.cancelled, 2);
        assert_eq!(report.skipped, 0);
        let statuses = store
            .continuations(&session_id)
            .expect("continuations")
            .into_iter()
            .map(|continuation| {
                (
                    continuation.continuation_id,
                    continuation.status,
                    continuation.error,
                )
            })
            .collect::<Vec<_>>();
        assert!(statuses.iter().any(|(id, status, error)| {
            id == &queued.continuation_id
                && *status == SessionContinuationStatus::Cancelled
                && error.as_deref() == Some("operator requested cancel")
        }));
        assert!(statuses.iter().any(|(id, status, error)| {
            id == &running.continuation_id
                && *status == SessionContinuationStatus::Cancelled
                && error.as_deref() == Some("operator requested cancel")
        }));
        let cancel_events = store
            .agent_events(&session_id)
            .expect("cancel events")
            .into_iter()
            .filter(|event| matches!(event.kind, AgentEventKind::ContinuationCancelled))
            .collect::<Vec<_>>();
        assert_eq!(cancel_events.len(), 2);
        assert!(cancel_events.iter().any(|event| {
            event.payload["continuation_id"] == queued.continuation_id.as_str()
                && event.payload["reason"] == "operator requested cancel"
                && event.payload["status"] == "cancelled"
        }));
        assert!(cancel_events.iter().any(|event| {
            event.payload["continuation_id"] == running.continuation_id.as_str()
                && event.payload["reason"] == "operator requested cancel"
                && event.payload["status"] == "cancelled"
        }));
    }

    #[test]
    fn workbench_evidence_entry_uses_active_leaf_parent_and_payload_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("evidence-session");
        let mut root = SessionEntry::new(session_id.clone(), SessionEntryKind::UserMessage);
        root.visible_text = Some("inspect status".to_owned());
        store.append_entry(&root).expect("root entry");

        let entry = append_workbench_evidence_entry(
            &store,
            &session_id,
            "default-agent",
            Path::new("C:/Workspace/example"),
            "provider",
            "workbench provider status queried",
            json!({"args": ["inspect"]}),
        )
        .expect("append evidence entry");

        assert_eq!(entry.kind, SessionEntryKind::Custom);
        assert_eq!(entry.parent_entry_id.as_ref(), Some(&root.entry_id));
        assert_eq!(
            entry.visible_text.as_deref(),
            Some("workbench provider status queried")
        );
        assert_eq!(entry.payload["operation"], "workbench_evidence");
        assert_eq!(entry.payload["kind"], "provider");
        assert_eq!(entry.payload["session_id"], "evidence-session");
        assert_eq!(entry.payload["agent_id"], "default-agent");
        assert_eq!(entry.payload["data"]["args"][0], "inspect");
        let persisted = store
            .session_entry(&entry.entry_id)
            .expect("read evidence entry")
            .expect("persisted evidence entry");
        assert_eq!(persisted.parent_entry_id.as_ref(), Some(&root.entry_id));
    }

    #[test]
    fn workbench_fork_branches_from_active_leaf_with_payload_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("fork-session");
        let mut root = SessionEntry::new(session_id.clone(), SessionEntryKind::UserMessage);
        root.visible_text = Some("original request".to_owned());
        store.append_entry(&root).expect("root entry");

        let report = fork_workbench_session(
            &store,
            &session_id,
            "default-agent",
            Path::new("C:/Workspace/example"),
            "try alternate implementation",
        )
        .expect("fork session");

        let WorkbenchForkReport::Forked {
            parent_entry_id,
            entry,
            summary,
            ..
        } = report
        else {
            panic!("expected forked report");
        };
        assert_eq!(parent_entry_id, root.entry_id);
        assert_eq!(entry.parent_entry_id.as_ref(), Some(&root.entry_id));
        assert_eq!(entry.kind, SessionEntryKind::BranchSummary);
        assert_eq!(summary, "try alternate implementation");
        assert_eq!(entry.payload["operation"], "branch_summary");
        assert_eq!(entry.payload["data"]["source"], "workbench");
        assert_eq!(entry.payload["data"]["command"], "/fork");
        assert_eq!(entry.payload["data"]["agent_id"], "default-agent");
        let persisted = store
            .session_entry(&entry.entry_id)
            .expect("read branch entry")
            .expect("persisted branch entry");
        assert_eq!(persisted.parent_entry_id.as_ref(), Some(&root.entry_id));
    }

    #[test]
    fn workbench_fork_reports_missing_session_or_leaf_without_mutating() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let missing_id = SessionId::from("missing-session");
        let missing = fork_workbench_session(
            &store,
            &missing_id,
            "default-agent",
            Path::new("C:/Workspace/example"),
            "summary",
        )
        .expect("missing report");
        assert_eq!(
            missing,
            WorkbenchForkReport::SessionNotFound {
                session_id: missing_id
            }
        );

        let empty_id = SessionId::from("empty-session");
        store
            .upsert_session(&SessionRecord::new(empty_id.clone(), SessionSource::Cli))
            .expect("empty session");
        let empty = fork_workbench_session(
            &store,
            &empty_id,
            "default-agent",
            Path::new("C:/Workspace/example"),
            "summary",
        )
        .expect("missing active leaf report");
        assert_eq!(
            empty,
            WorkbenchForkReport::MissingActiveLeaf {
                session_id: empty_id
            }
        );
    }

    #[test]
    fn workbench_requeue_moves_failed_continuation_back_to_queued_with_payload_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("requeue-session");
        let continuation = store
            .enqueue_continuation(&SessionContinuationInput::new(
                session_id,
                SessionContinuationKind::NextTurn,
            ))
            .expect("enqueue continuation");
        store
            .fail_continuation(&continuation.continuation_id, "model failed")
            .expect("failed continuation")
            .expect("failed continuation exists");

        let requeued = requeue_workbench_continuation(&store, &continuation.continuation_id)
            .expect("requeue continuation")
            .expect("requeued continuation");

        assert_eq!(requeued.status, SessionContinuationStatus::Queued);
        assert_eq!(requeued.error.as_deref(), Some("workbench requeue"));
        assert_eq!(requeued.payload["requeued_by"], "workbench");
        assert_eq!(requeued.payload["requeue_source"], "/queue retry");
    }
}
