// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn admit_input(
    conn: &Connection,
    path: &Path,
    input: &SessionInputAdmission,
) -> Result<SessionInput> {
    insert_missing_session(conn, path, &input.session_id)?;
    let admitted = SessionInput {
        input_id: SessionInputId::new(),
        session_id: input.session_id.clone(),
        status: SessionInputStatus::Admitted,
        idempotency_key_digest: input.idempotency_key_digest.clone(),
        payload: input.payload.clone(),
        admitted_at: OffsetDateTime::now_utc(),
        promoted_turn_id: None,
        promoted_at: None,
        cancelled_at: None,
        cancel_reason: None,
    };
    insert_session_input(conn, path, &admitted)?;
    Ok(admitted)
}

pub(super) fn promote_input(
    conn: &Connection,
    path: &Path,
    input_id: &SessionInputId,
    turn_id: &TurnId,
) -> Result<Option<SessionInput>> {
    let promoted_at = format_time(OffsetDateTime::now_utc())?;
    let updated = conn
        .execute(
            r#"
            UPDATE session_inputs
            SET status = ?1,
                promoted_turn_id = ?2,
                promoted_at = ?3
            WHERE id = ?4
              AND status = ?5
            "#,
            params![
                session_input_status_to_str(SessionInputStatus::Promoted),
                turn_id.as_str(),
                promoted_at,
                input_id.as_str(),
                session_input_status_to_str(SessionInputStatus::Admitted),
            ],
        )
        .map_err(|source| sqlite_error(path, source))?;
    if updated == 0 {
        return Ok(None);
    }
    session_input(conn, path, input_id)
}

pub(super) fn cancel_input(
    conn: &Connection,
    path: &Path,
    input_id: &SessionInputId,
    reason: &str,
) -> Result<Option<SessionInput>> {
    let cancelled_at = format_time(OffsetDateTime::now_utc())?;
    let updated = conn
        .execute(
            r#"
            UPDATE session_inputs
            SET status = ?1,
                cancelled_at = ?2,
                cancel_reason = ?3
            WHERE id = ?4
              AND status = ?5
            "#,
            params![
                session_input_status_to_str(SessionInputStatus::Cancelled),
                cancelled_at,
                reason,
                input_id.as_str(),
                session_input_status_to_str(SessionInputStatus::Admitted),
            ],
        )
        .map_err(|source| sqlite_error(path, source))?;
    if updated == 0 {
        return Ok(None);
    }
    session_input(conn, path, input_id)
}

pub(super) fn session_inputs(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
) -> Result<Vec<SessionInput>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, session_id, status, idempotency_key_digest, payload_json,
                   admitted_at, promoted_turn_id, promoted_at, cancelled_at, cancel_reason
            FROM session_inputs
            WHERE session_id = ?1
            ORDER BY admitted_at ASC, rowid ASC
            "#,
        )
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map(params![session_id.as_str()], session_input_row)
        .map_err(|source| sqlite_error(path, source))?;
    let mut inputs = Vec::new();
    for row in rows {
        inputs.push(session_input_from_parts(
            row.map_err(|source| sqlite_error(path, source))?,
        )?);
    }
    Ok(inputs)
}

fn insert_session_input(conn: &Connection, path: &Path, input: &SessionInput) -> Result<()> {
    let payload_json = serde_json::to_string(&input.payload)?;
    let admitted_at = format_time(input.admitted_at)?;
    let promoted_at = input.promoted_at.map(format_time).transpose()?;
    let cancelled_at = input.cancelled_at.map(format_time).transpose()?;
    conn.execute(
        r#"
        INSERT INTO session_inputs (
            id, session_id, status, idempotency_key_digest, payload_json,
            admitted_at, promoted_turn_id, promoted_at, cancelled_at, cancel_reason
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
        params![
            input.input_id.as_str(),
            input.session_id.as_str(),
            session_input_status_to_str(input.status),
            input.idempotency_key_digest.as_deref(),
            payload_json,
            admitted_at,
            input.promoted_turn_id.as_ref().map(TurnId::as_str),
            promoted_at.as_deref(),
            cancelled_at.as_deref(),
            input.cancel_reason.as_deref(),
        ],
    )
    .map_err(|source| sqlite_error(path, source))?;
    Ok(())
}

fn session_input(
    conn: &Connection,
    path: &Path,
    input_id: &SessionInputId,
) -> Result<Option<SessionInput>> {
    conn.query_row(
        r#"
        SELECT id, session_id, status, idempotency_key_digest, payload_json,
               admitted_at, promoted_turn_id, promoted_at, cancelled_at, cancel_reason
        FROM session_inputs
        WHERE id = ?1
        "#,
        params![input_id.as_str()],
        session_input_row,
    )
    .optional()
    .map_err(|source| sqlite_error(path, source))?
    .map(session_input_from_parts)
    .transpose()
}

fn session_input_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionInputRow> {
    Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, Option<String>>(3)?,
        row.get::<_, String>(4)?,
        row.get::<_, String>(5)?,
        row.get::<_, Option<String>>(6)?,
        row.get::<_, Option<String>>(7)?,
        row.get::<_, Option<String>>(8)?,
        row.get::<_, Option<String>>(9)?,
    ))
}
