// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) type ApprovalRecordRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
);

pub(super) type AgentEventRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
);

pub(super) fn agent_event_from_parts(row: AgentEventRow) -> Result<AgentEvent> {
    let (id, session_id, turn_id, parent_event_id, at, source, kind_json, payload_json) = row;
    Ok(AgentEvent {
        event_id: id.into(),
        session_id: session_id.into(),
        turn_id: turn_id.into(),
        parent_event_id: parent_event_id.map(Into::into),
        at: parse_time(&at)?,
        source: event_source_from_str(&source)?,
        kind: serde_json::from_str::<AgentEventKind>(&kind_json)?,
        payload: serde_json::from_str(&payload_json)?,
    })
}

pub(super) fn approval_record_from_parts(row: ApprovalRecordRow) -> Result<ApprovalRecord> {
    let (approval_id, session_id, turn_id, at, status, request_json, decision_json) = row;
    Ok(ApprovalRecord {
        approval_id,
        session_id: session_id.into(),
        turn_id: turn_id.map(Into::into),
        at: parse_time(&at)?,
        status: approval_status_from_str(&status)?,
        request: serde_json::from_str(&request_json)?,
        decision: decision_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()?,
    })
}
pub(super) type SessionEntryRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
    String,
);

pub(super) fn session_entry_from_parts(row: SessionEntryRow) -> Result<SessionEntry> {
    let (id, session_id, parent_entry_id, turn_id, at, kind, visible_text, payload_json) = row;
    Ok(SessionEntry {
        entry_id: SessionEntryId::from(id),
        session_id: SessionId::from(session_id),
        parent_entry_id: parent_entry_id.map(SessionEntryId::from),
        turn_id: turn_id.map(TurnId::from),
        at: parse_time(&at)?,
        kind: entry_kind_from_str(&kind)?,
        visible_text,
        payload: serde_json::from_str(&payload_json)?,
    })
}

pub(super) type SessionInputRow = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub(super) fn session_input_from_parts(row: SessionInputRow) -> Result<SessionInput> {
    let (
        id,
        session_id,
        status,
        idempotency_key_digest,
        payload_json,
        admitted_at,
        promoted_turn_id,
        promoted_at,
        cancelled_at,
        cancel_reason,
    ) = row;
    Ok(SessionInput {
        input_id: SessionInputId::from(id),
        session_id: SessionId::from(session_id),
        status: session_input_status_from_str(&status)?,
        idempotency_key_digest,
        payload: serde_json::from_str(&payload_json)?,
        admitted_at: parse_time(&admitted_at)?,
        promoted_turn_id: promoted_turn_id.map(TurnId::from),
        promoted_at: promoted_at.as_deref().map(parse_time).transpose()?,
        cancelled_at: cancelled_at.as_deref().map(parse_time).transpose()?,
        cancel_reason,
    })
}

pub(super) type SessionTurnRow = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub(super) fn session_turn_from_parts(row: SessionTurnRow) -> Result<SessionTurnRecord> {
    let (
        session_id,
        turn_id,
        status,
        started_at,
        updated_at,
        completed_at,
        lease_owner,
        lease_expires_at,
        error,
    ) = row;
    Ok(SessionTurnRecord {
        session_id: SessionId::from(session_id),
        turn_id: TurnId::from(turn_id),
        status: session_turn_status_from_str(&status)?,
        started_at: parse_time(&started_at)?,
        updated_at: parse_time(&updated_at)?,
        completed_at: completed_at.as_deref().map(parse_time).transpose()?,
        lease_owner,
        lease_expires_at: lease_expires_at.as_deref().map(parse_time).transpose()?,
        error,
    })
}
pub(super) type SessionContinuationRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
    i64,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
);

pub(super) fn session_continuation_from_parts(
    row: SessionContinuationRow,
) -> Result<SessionContinuation> {
    let (
        id,
        session_id,
        turn_id,
        parent_continuation_id,
        kind,
        status,
        status_reason,
        priority,
        payload_json,
        created_at,
        updated_at,
        claimed_at,
        completed_at,
        lease_owner,
        lease_expires_at,
        attempt_count,
        error,
    ) = row;
    Ok(SessionContinuation {
        continuation_id: ContinuationId::from(id),
        session_id: SessionId::from(session_id),
        turn_id: turn_id.map(TurnId::from),
        parent_continuation_id: parent_continuation_id.map(ContinuationId::from),
        kind: continuation_kind_from_str(&kind)?,
        status: continuation_status_from_str(&status)?,
        status_reason: status_reason
            .as_deref()
            .map(continuation_status_reason_from_str)
            .transpose()?,
        priority,
        payload: serde_json::from_str(&payload_json)?,
        created_at: parse_time(&created_at)?,
        updated_at: parse_time(&updated_at)?,
        claimed_at: claimed_at.as_deref().map(parse_time).transpose()?,
        completed_at: completed_at.as_deref().map(parse_time).transpose()?,
        lease_owner,
        lease_expires_at: lease_expires_at.as_deref().map(parse_time).transpose()?,
        attempt_count,
        error,
    })
}
pub(super) fn format_time(value: OffsetDateTime) -> Result<String> {
    value.format(&Rfc3339).map_err(IkarosError::Time)
}

pub(super) fn parse_time(value: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|source| IkarosError::Message(format!("invalid state.db timestamp: {source}")))
}

pub(super) fn entry_kind_to_str(kind: SessionEntryKind) -> &'static str {
    match kind {
        SessionEntryKind::SystemMessage => "system_message",
        SessionEntryKind::UserMessage => "user_message",
        SessionEntryKind::AssistantMessage => "assistant_message",
        SessionEntryKind::ToolResult => "tool_result",
        SessionEntryKind::ModelChange => "model_change",
        SessionEntryKind::Compaction => "compaction",
        SessionEntryKind::BranchSummary => "branch_summary",
        SessionEntryKind::Custom => "custom",
        SessionEntryKind::Leaf => "leaf",
    }
}

pub(super) fn entry_kind_from_str(value: &str) -> Result<SessionEntryKind> {
    match value {
        "system_message" => Ok(SessionEntryKind::SystemMessage),
        "user_message" => Ok(SessionEntryKind::UserMessage),
        "assistant_message" => Ok(SessionEntryKind::AssistantMessage),
        "tool_result" => Ok(SessionEntryKind::ToolResult),
        "model_change" => Ok(SessionEntryKind::ModelChange),
        "compaction" => Ok(SessionEntryKind::Compaction),
        "branch_summary" => Ok(SessionEntryKind::BranchSummary),
        "custom" => Ok(SessionEntryKind::Custom),
        "leaf" => Ok(SessionEntryKind::Leaf),
        other => Err(IkarosError::Message(format!(
            "unknown session entry kind in state.db: {other}"
        ))),
    }
}

pub(super) fn continuation_kind_to_str(kind: SessionContinuationKind) -> &'static str {
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

pub(super) fn continuation_kind_from_str(value: &str) -> Result<SessionContinuationKind> {
    match value {
        "steer" => Ok(SessionContinuationKind::Steer),
        "follow_up" => Ok(SessionContinuationKind::FollowUp),
        "next_turn" => Ok(SessionContinuationKind::NextTurn),
        "resume" => Ok(SessionContinuationKind::Resume),
        "retry" => Ok(SessionContinuationKind::Retry),
        "compact" => Ok(SessionContinuationKind::Compact),
        "tool_result" => Ok(SessionContinuationKind::ToolResult),
        other => Err(IkarosError::Message(format!(
            "unknown session continuation kind in state.db: {other}"
        ))),
    }
}

pub(super) fn continuation_status_to_str(status: SessionContinuationStatus) -> &'static str {
    match status {
        SessionContinuationStatus::Queued => "queued",
        SessionContinuationStatus::Running => "running",
        SessionContinuationStatus::Completed => "completed",
        SessionContinuationStatus::Failed => "failed",
        SessionContinuationStatus::Cancelled => "cancelled",
    }
}

pub(super) fn continuation_status_from_str(value: &str) -> Result<SessionContinuationStatus> {
    match value {
        "queued" => Ok(SessionContinuationStatus::Queued),
        "running" => Ok(SessionContinuationStatus::Running),
        "completed" => Ok(SessionContinuationStatus::Completed),
        "failed" => Ok(SessionContinuationStatus::Failed),
        "cancelled" => Ok(SessionContinuationStatus::Cancelled),
        other => Err(IkarosError::Message(format!(
            "unknown session continuation status in state.db: {other}"
        ))),
    }
}

pub(super) fn continuation_status_reason_to_str(
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

pub(super) fn continuation_status_reason_from_str(
    value: &str,
) -> Result<SessionContinuationStatusReason> {
    match value {
        "enqueued" => Ok(SessionContinuationStatusReason::Enqueued),
        "claimed" => Ok(SessionContinuationStatusReason::Claimed),
        "completed" => Ok(SessionContinuationStatusReason::Completed),
        "failed" => Ok(SessionContinuationStatusReason::Failed),
        "cancelled" => Ok(SessionContinuationStatusReason::Cancelled),
        "requeued" => Ok(SessionContinuationStatusReason::Requeued),
        "lease_expired" => Ok(SessionContinuationStatusReason::LeaseExpired),
        other => Err(IkarosError::Message(format!(
            "unknown session continuation status reason in state.db: {other}"
        ))),
    }
}

pub(super) fn continuation_status_reason_for_status(
    status: SessionContinuationStatus,
) -> SessionContinuationStatusReason {
    match status {
        SessionContinuationStatus::Queued => SessionContinuationStatusReason::Requeued,
        SessionContinuationStatus::Running => SessionContinuationStatusReason::Claimed,
        SessionContinuationStatus::Completed => SessionContinuationStatusReason::Completed,
        SessionContinuationStatus::Failed => SessionContinuationStatusReason::Failed,
        SessionContinuationStatus::Cancelled => SessionContinuationStatusReason::Cancelled,
    }
}

pub(super) fn session_input_status_to_str(status: SessionInputStatus) -> &'static str {
    match status {
        SessionInputStatus::Admitted => "admitted",
        SessionInputStatus::Promoted => "promoted",
        SessionInputStatus::Cancelled => "cancelled",
    }
}

pub(super) fn session_input_status_from_str(value: &str) -> Result<SessionInputStatus> {
    match value {
        "admitted" => Ok(SessionInputStatus::Admitted),
        "promoted" => Ok(SessionInputStatus::Promoted),
        "cancelled" => Ok(SessionInputStatus::Cancelled),
        other => Err(IkarosError::Message(format!(
            "unknown session input status in state.db: {other}"
        ))),
    }
}

pub(super) fn session_turn_status_to_str(status: SessionTurnStatus) -> &'static str {
    match status {
        SessionTurnStatus::Pending => "pending",
        SessionTurnStatus::Running => "running",
        SessionTurnStatus::Completed => "completed",
        SessionTurnStatus::Failed => "failed",
        SessionTurnStatus::Cancelled => "cancelled",
    }
}

pub(super) fn session_turn_status_from_str(value: &str) -> Result<SessionTurnStatus> {
    match value {
        "pending" => Ok(SessionTurnStatus::Pending),
        "running" => Ok(SessionTurnStatus::Running),
        "completed" => Ok(SessionTurnStatus::Completed),
        "failed" => Ok(SessionTurnStatus::Failed),
        "cancelled" => Ok(SessionTurnStatus::Cancelled),
        other => Err(IkarosError::Message(format!(
            "unknown session turn status in state.db: {other}"
        ))),
    }
}

pub(super) fn event_source_to_str(source: crate::AgentEventSource) -> &'static str {
    match source {
        crate::AgentEventSource::Runtime => "runtime",
        crate::AgentEventSource::User => "user",
        crate::AgentEventSource::Model => "model",
        crate::AgentEventSource::Tool => "tool",
        crate::AgentEventSource::Harness => "harness",
        crate::AgentEventSource::Context => "context",
        crate::AgentEventSource::Memory => "memory",
        crate::AgentEventSource::Audit => "audit",
    }
}

pub(super) fn event_source_from_str(value: &str) -> Result<crate::AgentEventSource> {
    match value {
        "runtime" => Ok(crate::AgentEventSource::Runtime),
        "user" => Ok(crate::AgentEventSource::User),
        "model" => Ok(crate::AgentEventSource::Model),
        "tool" => Ok(crate::AgentEventSource::Tool),
        "harness" => Ok(crate::AgentEventSource::Harness),
        "context" => Ok(crate::AgentEventSource::Context),
        "memory" => Ok(crate::AgentEventSource::Memory),
        "audit" => Ok(crate::AgentEventSource::Audit),
        other => Err(IkarosError::Message(format!(
            "unknown agent event source in state.db: {other}"
        ))),
    }
}

pub(super) fn approval_status_to_str(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Requested => "requested",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Expired => "expired",
        ApprovalStatus::Executed => "executed",
    }
}

pub(super) fn approval_status_from_str(value: &str) -> Result<ApprovalStatus> {
    match value {
        "requested" => Ok(ApprovalStatus::Requested),
        "approved" => Ok(ApprovalStatus::Approved),
        "denied" => Ok(ApprovalStatus::Denied),
        "expired" => Ok(ApprovalStatus::Expired),
        "executed" => Ok(ApprovalStatus::Executed),
        other => Err(IkarosError::Message(format!(
            "unknown approval status in state.db: {other}"
        ))),
    }
}

pub(super) fn database_has_user_schema(conn: &Connection, path: &Path) -> Result<bool> {
    let existing = conn
        .query_row(
            r#"
            SELECT name
            FROM sqlite_schema
            WHERE type IN ('table', 'index', 'trigger', 'view')
              AND name NOT LIKE 'sqlite_%'
            LIMIT 1
            "#,
            [],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|source| sqlite_error(path, source))?;
    Ok(existing.is_some())
}

pub(super) fn sqlite_error(path: &Path, source: rusqlite::Error) -> IkarosError {
    sqlite_error_for_operation(path, "query", source)
}

pub(super) fn sqlite_error_for_operation(
    path: &Path,
    operation: &'static str,
    source: rusqlite::Error,
) -> IkarosError {
    IkarosError::Message(format!(
        "sqlite {} error at {} during {operation}: {source}",
        sqlite_error_class(&source),
        path.display()
    ))
}

pub(super) fn sqlite_error_class(source: &rusqlite::Error) -> &'static str {
    match source {
        rusqlite::Error::SqliteFailure(error, _) => match error.code {
            rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked => "busy",
            rusqlite::ErrorCode::ConstraintViolation => "constraint",
            rusqlite::ErrorCode::DiskFull
            | rusqlite::ErrorCode::CannotOpen
            | rusqlite::ErrorCode::ReadOnly => "storage",
            _ => "query",
        },
        _ => "query",
    }
}
