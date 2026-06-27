// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn upsert_turn(conn: &Connection, path: &Path, turn: &SessionTurnRecord) -> Result<()> {
    insert_missing_session(conn, path, &turn.session_id)?;
    let started_at = format_time(turn.started_at)?;
    let updated_at = format_time(turn.updated_at)?;
    let completed_at = turn.completed_at.map(format_time).transpose()?;
    let lease_expires_at = turn.lease_expires_at.map(format_time).transpose()?;
    conn.execute(
        r#"
        INSERT INTO session_turns (
            session_id, turn_id, status, started_at, updated_at, completed_at,
            lease_owner, lease_expires_at, error
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ON CONFLICT(session_id, turn_id) DO UPDATE SET
            status = excluded.status,
            updated_at = excluded.updated_at,
            completed_at = excluded.completed_at,
            lease_owner = excluded.lease_owner,
            lease_expires_at = excluded.lease_expires_at,
            error = excluded.error
        "#,
        params![
            turn.session_id.as_str(),
            turn.turn_id.as_str(),
            session_turn_status_to_str(turn.status),
            started_at,
            updated_at,
            completed_at.as_deref(),
            turn.lease_owner.as_deref(),
            lease_expires_at.as_deref(),
            turn.error.as_deref(),
        ],
    )
    .map_err(|source| sqlite_error(path, source))?;
    Ok(())
}

pub(super) fn session_turn(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
    turn_id: &TurnId,
) -> Result<Option<SessionTurnRecord>> {
    conn.query_row(
        r#"
        SELECT session_id, turn_id, status, started_at, updated_at, completed_at,
               lease_owner, lease_expires_at, error
        FROM session_turns
        WHERE session_id = ?1
          AND turn_id = ?2
        "#,
        params![session_id.as_str(), turn_id.as_str()],
        session_turn_row,
    )
    .optional()
    .map_err(|source| sqlite_error(path, source))?
    .map(session_turn_from_parts)
    .transpose()
}

pub(super) fn session_turns(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
) -> Result<Vec<SessionTurnRecord>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT session_id, turn_id, status, started_at, updated_at, completed_at,
                   lease_owner, lease_expires_at, error
            FROM session_turns
            WHERE session_id = ?1
            ORDER BY started_at ASC, rowid ASC
            "#,
        )
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map(params![session_id.as_str()], session_turn_row)
        .map_err(|source| sqlite_error(path, source))?;
    let mut turns = Vec::new();
    for row in rows {
        turns.push(session_turn_from_parts(
            row.map_err(|source| sqlite_error(path, source))?,
        )?);
    }
    Ok(turns)
}

fn session_turn_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionTurnRow> {
    Ok((
        row.get::<_, String>(0)?,
        row.get::<_, String>(1)?,
        row.get::<_, String>(2)?,
        row.get::<_, String>(3)?,
        row.get::<_, String>(4)?,
        row.get::<_, Option<String>>(5)?,
        row.get::<_, Option<String>>(6)?,
        row.get::<_, Option<String>>(7)?,
        row.get::<_, Option<String>>(8)?,
    ))
}
