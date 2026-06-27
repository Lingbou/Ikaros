// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn append_approval(
    conn: &Connection,
    path: &Path,
    approval: &ApprovalRecord,
) -> Result<()> {
    insert_missing_session(conn, path, &approval.session_id)?;
    conn.execute(
        r#"
        INSERT INTO approvals (
            id, session_id, turn_id, at, status, request_json, decision_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        ON CONFLICT(id) DO UPDATE SET
            session_id = excluded.session_id,
            turn_id = excluded.turn_id,
            at = excluded.at,
            status = excluded.status,
            request_json = excluded.request_json,
            decision_json = excluded.decision_json
        "#,
        params![
            approval.approval_id.as_str(),
            approval.session_id.as_str(),
            approval.turn_id.as_ref().map(TurnId::as_str),
            format_time(approval.at)?,
            approval_status_to_str(approval.status),
            serde_json::to_string(&approval.request)?,
            approval
                .decision
                .as_ref()
                .map(serde_json::to_string)
                .transpose()?,
        ],
    )
    .map_err(|source| sqlite_error(path, source))?;
    record_approval_timeline_item(conn, path, approval)?;
    Ok(())
}

pub(super) fn approval_record(
    conn: &Connection,
    path: &Path,
    approval_id: &str,
) -> Result<Option<ApprovalRecord>> {
    conn.query_row(
        r#"
        SELECT id, session_id, turn_id, at, status, request_json, decision_json
        FROM approvals
        WHERE id = ?1
        "#,
        params![approval_id],
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
    .optional()
    .map_err(|source| sqlite_error(path, source))?
    .map(approval_record_from_parts)
    .transpose()
}
