// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn continuation_by_id(
    conn: &Connection,
    path: &Path,
    continuation_id: &ContinuationId,
) -> Result<Option<SessionContinuation>> {
    conn.query_row(
        r#"
        SELECT id, session_id, turn_id, parent_continuation_id, kind, status, status_reason, priority,
               payload_json, created_at, updated_at, claimed_at, completed_at, lease_owner,
               lease_expires_at, attempt_count, error
        FROM session_continuations
        WHERE id = ?1
        "#,
        params![continuation_id.as_str()],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, i64>(15)?,
                row.get::<_, Option<String>>(16)?,
            ))
        },
    )
    .optional()
    .map_err(|source| sqlite_error(path, source))?
    .map(session_continuation_from_parts)
    .transpose()
}

pub(super) fn enqueue_continuation(
    conn: &Connection,
    path: &Path,
    input: &SessionContinuationInput,
) -> Result<SessionContinuation> {
    insert_missing_session(conn, path, &input.session_id)?;
    let continuation_id = ContinuationId::new();
    let now = OffsetDateTime::now_utc();
    conn.execute(
        r#"
        INSERT INTO session_continuations (
            id, session_id, turn_id, parent_continuation_id, kind, status, status_reason,
            priority, payload_json, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        "#,
        params![
            continuation_id.as_str(),
            input.session_id.as_str(),
            input.turn_id.as_ref().map(TurnId::as_str),
            input
                .parent_continuation_id
                .as_ref()
                .map(ContinuationId::as_str),
            continuation_kind_to_str(input.kind),
            continuation_status_to_str(SessionContinuationStatus::Queued),
            continuation_status_reason_to_str(SessionContinuationStatusReason::Enqueued),
            input.priority,
            serde_json::to_string(&input.payload)?,
            format_time(now)?,
            format_time(now)?,
        ],
    )
    .map_err(|source| sqlite_error(path, source))?;
    continuation_by_id(conn, path, &continuation_id)?.ok_or_else(|| {
        IkarosError::Message(format!(
            "queued continuation disappeared after insert: {continuation_id}"
        ))
    })
}

pub(super) fn claim_next_continuation_in_transaction(
    conn: &Connection,
    path: &Path,
    claim: &SessionContinuationClaim,
) -> Result<Option<SessionContinuation>> {
    reclaim_expired_continuations(conn, path)?;
    let mut queued = queued_continuations(conn, path)?;
    queued.retain(|continuation| continuation_matches_claim(continuation, claim));
    queued.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.continuation_id.cmp(&right.continuation_id))
    });
    let Some(selected) = queued.into_iter().next() else {
        return Ok(None);
    };
    let now = OffsetDateTime::now_utc();
    let lease_duration = claim
        .lease_duration_seconds
        .unwrap_or(DEFAULT_CONTINUATION_LEASE_SECONDS)
        .max(0);
    let lease_expires_at = now + time::Duration::seconds(lease_duration);
    let claim_status_reason =
        if selected.status_reason == Some(SessionContinuationStatusReason::LeaseExpired) {
            SessionContinuationStatusReason::LeaseExpired
        } else {
            SessionContinuationStatusReason::Claimed
        };
    let claim_error = if claim_status_reason == SessionContinuationStatusReason::LeaseExpired {
        selected.error.as_deref()
    } else {
        None
    };
    let updated = conn
        .execute(
            r#"
            UPDATE session_continuations
            SET status = ?1,
                status_reason = ?2,
                updated_at = ?3,
                claimed_at = ?3,
                lease_owner = ?4,
                lease_expires_at = ?5,
                attempt_count = attempt_count + 1,
                error = ?6
            WHERE id = ?7
              AND status = ?8
            "#,
            params![
                continuation_status_to_str(SessionContinuationStatus::Running),
                continuation_status_reason_to_str(claim_status_reason),
                format_time(now)?,
                claim.lease_owner.as_deref(),
                format_time(lease_expires_at)?,
                claim_error,
                selected.continuation_id.as_str(),
                continuation_status_to_str(SessionContinuationStatus::Queued),
            ],
        )
        .map_err(|source| sqlite_error(path, source))?;
    if updated == 0 {
        return Ok(None);
    }
    continuation_by_id(conn, path, &selected.continuation_id)
}

pub(super) fn reclaim_expired_continuations(conn: &Connection, path: &Path) -> Result<usize> {
    let now = format_time(OffsetDateTime::now_utc())?;
    conn.execute(
        r#"
        UPDATE session_continuations
        SET status = ?1,
            status_reason = ?2,
            updated_at = ?3,
            lease_owner = NULL,
            lease_expires_at = NULL,
            error = COALESCE(error, 'lease expired')
        WHERE status = ?4
          AND lease_expires_at IS NOT NULL
          AND lease_expires_at <= ?3
        "#,
        params![
            continuation_status_to_str(SessionContinuationStatus::Queued),
            continuation_status_reason_to_str(SessionContinuationStatusReason::LeaseExpired),
            now,
            continuation_status_to_str(SessionContinuationStatus::Running),
        ],
    )
    .map_err(|source| sqlite_error(path, source))
}

pub(super) fn queued_continuations(
    conn: &Connection,
    path: &Path,
) -> Result<Vec<SessionContinuation>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, session_id, turn_id, parent_continuation_id, kind, status, status_reason, priority,
                   payload_json, created_at, updated_at, claimed_at, completed_at, lease_owner,
                   lease_expires_at, attempt_count, error
            FROM session_continuations
            WHERE status = ?1
            ORDER BY priority ASC, created_at ASC, rowid ASC
            "#,
        )
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map(
            params![continuation_status_to_str(
                SessionContinuationStatus::Queued
            )],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, i64>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, i64>(15)?,
                    row.get::<_, Option<String>>(16)?,
                ))
            },
        )
        .map_err(|source| sqlite_error(path, source))?;
    let mut continuations = Vec::new();
    for row in rows {
        continuations.push(session_continuation_from_parts(
            row.map_err(|source| sqlite_error(path, source))?,
        )?);
    }
    Ok(continuations)
}

pub(super) fn continuation_matches_claim(
    continuation: &SessionContinuation,
    claim: &SessionContinuationClaim,
) -> bool {
    if let Some(session_id) = claim.session_id.as_ref() {
        if &continuation.session_id != session_id {
            return false;
        }
    }
    if let Some(turn_id) = claim.turn_id.as_ref() {
        if continuation.turn_id.as_ref() != Some(turn_id) {
            return false;
        }
    }
    claim.kinds.is_empty() || claim.kinds.contains(&continuation.kind)
}

pub(super) fn update_continuation_status(
    conn: &Connection,
    path: &Path,
    continuation_id: &ContinuationId,
    status: SessionContinuationStatus,
    payload: Option<serde_json::Value>,
    error: Option<&str>,
) -> Result<Option<SessionContinuation>> {
    let Some(existing) = continuation_by_id(conn, path, continuation_id)? else {
        return Ok(None);
    };
    let now = OffsetDateTime::now_utc();
    let payload = payload
        .map(|payload| merged_continuation_payload(existing.payload.clone(), payload))
        .unwrap_or(existing.payload);
    let completed_at = match status {
        SessionContinuationStatus::Completed
        | SessionContinuationStatus::Failed
        | SessionContinuationStatus::Cancelled => Some(format_time(now)?),
        SessionContinuationStatus::Queued | SessionContinuationStatus::Running => None,
    };
    conn.execute(
        r#"
        UPDATE session_continuations
        SET status = ?1,
            status_reason = ?2,
            payload_json = ?3,
            updated_at = ?4,
            completed_at = COALESCE(?5, completed_at),
            lease_expires_at = CASE
                WHEN ?5 IS NULL THEN lease_expires_at
                ELSE NULL
            END,
            error = ?6
        WHERE id = ?7
        "#,
        params![
            continuation_status_to_str(status),
            continuation_status_reason_to_str(continuation_status_reason_for_status(status)),
            serde_json::to_string(&payload)?,
            format_time(now)?,
            completed_at.as_deref(),
            error,
            continuation_id.as_str(),
        ],
    )
    .map_err(|source| sqlite_error(path, source))?;
    continuation_by_id(conn, path, continuation_id)
}

pub(super) fn requeue_continuation(
    conn: &Connection,
    path: &Path,
    continuation_id: &ContinuationId,
    reason: &str,
    payload: serde_json::Value,
) -> Result<Option<SessionContinuation>> {
    let Some(existing) = continuation_by_id(conn, path, continuation_id)? else {
        return Ok(None);
    };
    let merged_payload = if payload.is_null() {
        existing.payload
    } else {
        merged_continuation_payload(existing.payload, payload)
    };
    let now = OffsetDateTime::now_utc();
    let updated = conn
        .execute(
            r#"
        UPDATE session_continuations
        SET status = ?1,
            status_reason = ?2,
            payload_json = ?3,
            updated_at = ?4,
            completed_at = NULL,
            lease_owner = NULL,
            lease_expires_at = NULL,
            error = ?5
        WHERE id = ?6
          AND status IN (?7, ?8, ?9)
        "#,
            params![
                continuation_status_to_str(SessionContinuationStatus::Queued),
                continuation_status_reason_to_str(SessionContinuationStatusReason::Requeued),
                serde_json::to_string(&merged_payload)?,
                format_time(now)?,
                reason,
                continuation_id.as_str(),
                continuation_status_to_str(SessionContinuationStatus::Running),
                continuation_status_to_str(SessionContinuationStatus::Failed),
                continuation_status_to_str(SessionContinuationStatus::Cancelled),
            ],
        )
        .map_err(|source| sqlite_error(path, source))?;
    if updated == 0 {
        return Ok(None);
    }
    continuation_by_id(conn, path, continuation_id)
}

pub(super) fn merged_continuation_payload(
    mut existing: serde_json::Value,
    update: serde_json::Value,
) -> serde_json::Value {
    match (&mut existing, update) {
        (serde_json::Value::Object(existing), serde_json::Value::Object(update)) => {
            for (key, value) in update {
                existing.insert(key, value);
            }
            serde_json::Value::Object(existing.clone())
        }
        (_, update) => update,
    }
}

pub(super) fn continuations_for_session(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
) -> Result<Vec<SessionContinuation>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, session_id, turn_id, parent_continuation_id, kind, status, status_reason, priority,
                   payload_json, created_at, updated_at, claimed_at, completed_at, lease_owner,
                   lease_expires_at, attempt_count, error
            FROM session_continuations
            WHERE session_id = ?1
            ORDER BY created_at ASC, rowid ASC
            "#,
        )
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map(params![session_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, i64>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, Option<String>>(11)?,
                row.get::<_, Option<String>>(12)?,
                row.get::<_, Option<String>>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, i64>(15)?,
                row.get::<_, Option<String>>(16)?,
            ))
        })
        .map_err(|source| sqlite_error(path, source))?;
    let mut continuations = Vec::new();
    for row in rows {
        continuations.push(session_continuation_from_parts(
            row.map_err(|source| sqlite_error(path, source))?,
        )?);
    }
    Ok(continuations)
}
