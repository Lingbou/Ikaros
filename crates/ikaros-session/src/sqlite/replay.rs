// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn count_session_rows(
    conn: &Connection,
    path: &Path,
    table: &'static str,
    session_id: &SessionId,
) -> Result<usize> {
    let sql = format!("SELECT COUNT(*) FROM {table} WHERE session_id = ?1");
    let count = conn
        .query_row(&sql, params![session_id.as_str()], |row| {
            row.get::<_, i64>(0)
        })
        .map_err(|source| sqlite_error(path, source))?;
    Ok(count.max(0) as usize)
}

pub(super) fn session_entries_page(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
    offset: usize,
    limit: usize,
) -> Result<Vec<SessionEntry>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, session_id, parent_entry_id, turn_id, at, kind, visible_text, payload_json
            FROM session_entries
            WHERE session_id = ?1
            ORDER BY at ASC, rowid ASC
            LIMIT ?2 OFFSET ?3
            "#,
        )
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map(
            params![
                session_id.as_str(),
                sqlite_limit(limit),
                sqlite_offset(offset)
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .map_err(|source| sqlite_error(path, source))?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(session_entry_from_parts(
            row.map_err(|source| sqlite_error(path, source))?,
        )?);
    }
    Ok(entries)
}

pub(super) fn agent_events_page(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
    offset: usize,
    limit: usize,
) -> Result<Vec<AgentEvent>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, session_id, turn_id, parent_event_id, at, source, kind_json, payload_json
            FROM agent_events
            WHERE session_id = ?1
            ORDER BY event_seq ASC, rowid ASC
            LIMIT ?2 OFFSET ?3
            "#,
        )
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map(
            params![
                session_id.as_str(),
                sqlite_limit(limit),
                sqlite_offset(offset)
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .map_err(|source| sqlite_error(path, source))?;
    let mut events = Vec::new();
    for row in rows {
        events.push(agent_event_from_parts(
            row.map_err(|source| sqlite_error(path, source))?,
        )?);
    }
    Ok(events)
}

pub(super) fn approvals_page(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
    offset: usize,
    limit: usize,
) -> Result<Vec<ApprovalRecord>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, session_id, turn_id, at, status, request_json, decision_json
            FROM approvals
            WHERE session_id = ?1
            ORDER BY at ASC, rowid ASC
            LIMIT ?2 OFFSET ?3
            "#,
        )
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map(
            params![
                session_id.as_str(),
                sqlite_limit(limit),
                sqlite_offset(offset)
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            },
        )
        .map_err(|source| sqlite_error(path, source))?;
    let mut approvals = Vec::new();
    for row in rows {
        approvals.push(approval_record_from_parts(
            row.map_err(|source| sqlite_error(path, source))?,
        )?);
    }
    Ok(approvals)
}
