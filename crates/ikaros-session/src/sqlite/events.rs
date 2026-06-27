// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn append_agent_event(conn: &Connection, path: &Path, event: &AgentEvent) -> Result<()> {
    insert_missing_session(conn, path, &event.session_id)?;
    conn.execute(
        r#"
        INSERT INTO agent_events (
            id, session_id, turn_id, parent_event_id, event_seq, at, source, kind_json, payload_json
        )
        SELECT ?1, ?2, ?3, ?4, COALESCE(MAX(event_seq), 0) + 1, ?5, ?6, ?7, ?8
        FROM agent_events
        WHERE session_id = ?2
        "#,
        params![
            event.event_id.as_str(),
            event.session_id.as_str(),
            event.turn_id.as_str(),
            event.parent_event_id.as_ref().map(|id| id.as_str()),
            format_time(event.at)?,
            event_source_to_str(event.source),
            serde_json::to_string(&event.kind)?,
            serde_json::to_string(&event.payload)?,
        ],
    )
    .map_err(|source| sqlite_error(path, source))?;
    record_agent_event_timeline_item(conn, path, event)?;
    Ok(())
}

pub(super) fn agent_event(
    conn: &Connection,
    path: &Path,
    event_id: &str,
) -> Result<Option<AgentEvent>> {
    conn.query_row(
        r#"
        SELECT id, session_id, turn_id, parent_event_id, at, source, kind_json, payload_json
        FROM agent_events
        WHERE id = ?1
        "#,
        params![event_id],
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
    .optional()
    .map_err(|source| sqlite_error(path, source))?
    .map(agent_event_from_parts)
    .transpose()
}
