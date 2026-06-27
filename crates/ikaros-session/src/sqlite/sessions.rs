// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn upsert_session(
    conn: &Connection,
    path: &Path,
    session: &SessionRecord,
) -> Result<()> {
    let source_json = serde_json::to_string(&session.source)?;
    let started_at = format_time(session.started_at)?;
    let ended_at = session.ended_at.map(format_time).transpose()?;
    let workspace = session
        .workspace
        .as_ref()
        .map(|path| path.to_string_lossy().to_string());
    conn.execute(
        r#"
        INSERT INTO sessions (
            id, source_json, agent_id, workspace, parent_session_id,
            active_leaf_entry_id, started_at, ended_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(id) DO UPDATE SET
            source_json = excluded.source_json,
            agent_id = COALESCE(excluded.agent_id, sessions.agent_id),
            workspace = COALESCE(excluded.workspace, sessions.workspace),
            parent_session_id = COALESCE(excluded.parent_session_id, sessions.parent_session_id),
            active_leaf_entry_id = COALESCE(excluded.active_leaf_entry_id, sessions.active_leaf_entry_id),
            ended_at = COALESCE(excluded.ended_at, sessions.ended_at)
        "#,
        params![
            session.session_id.as_str(),
            source_json,
            session.agent_id.as_deref(),
            workspace.as_deref(),
            session.parent_session_id.as_ref().map(SessionId::as_str),
            session
                .active_leaf_entry_id
                .as_ref()
                .map(SessionEntryId::as_str),
            started_at,
            ended_at.as_deref(),
        ],
    )
    .map_err(|source| sqlite_error(path, source))?;
    Ok(())
}

pub(super) fn insert_missing_session(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
) -> Result<()> {
    let started_at = format_time(OffsetDateTime::now_utc())?;
    let source_json = serde_json::to_string(&SessionSource::Runtime)?;
    conn.execute(
        "INSERT OR IGNORE INTO sessions (id, source_json, started_at) VALUES (?1, ?2, ?3)",
        params![session_id.as_str(), source_json, started_at],
    )
    .map_err(|source| sqlite_error(path, source))?;
    Ok(())
}

pub(super) fn finish_session(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
    ended_at: OffsetDateTime,
) -> Result<()> {
    insert_missing_session(conn, path, session_id)?;
    conn.execute(
        "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
        params![format_time(ended_at)?, session_id.as_str()],
    )
    .map_err(|source| sqlite_error(path, source))?;
    Ok(())
}

pub(super) fn prune_ended_sessions_before(
    conn: &Connection,
    path: &Path,
    cutoff: OffsetDateTime,
) -> Result<SqlitePruneReport> {
    conn.execute_batch(
        r#"
        CREATE TEMP TABLE IF NOT EXISTS sessions_to_prune (
            id TEXT PRIMARY KEY
        );
        DELETE FROM sessions_to_prune;
        "#,
    )
    .map_err(|source| sqlite_error(path, source))?;
    let ended_before = format_time(cutoff)?;
    conn.execute(
        r#"
        INSERT INTO sessions_to_prune (id)
        SELECT id
        FROM sessions
        WHERE ended_at IS NOT NULL
          AND ended_at < ?1
        "#,
        params![&ended_before],
    )
    .map_err(|source| sqlite_error(path, source))?;
    let sessions_pruned = conn
        .query_row("SELECT COUNT(*) FROM sessions_to_prune", [], |row| {
            row.get::<_, usize>(0)
        })
        .map_err(|source| sqlite_error(path, source))?;
    let fts_rows = conn
        .execute(
            "DELETE FROM session_entries_fts WHERE session_id IN (SELECT id FROM sessions_to_prune)",
            [],
        )
        .map_err(|source| sqlite_error(path, source))?;
    let trigram_rows = conn
        .execute(
            "DELETE FROM session_entries_trigram WHERE session_id IN (SELECT id FROM sessions_to_prune)",
            [],
        )
        .map_err(|source| sqlite_error(path, source))?;
    let continuations_pruned = conn
        .execute(
            "DELETE FROM session_continuations WHERE session_id IN (SELECT id FROM sessions_to_prune)",
            [],
        )
        .map_err(|source| sqlite_error(path, source))?;
    let inputs_pruned = conn
        .execute(
            "DELETE FROM session_inputs WHERE session_id IN (SELECT id FROM sessions_to_prune)",
            [],
        )
        .map_err(|source| sqlite_error(path, source))?;
    let turns_pruned = conn
        .execute(
            "DELETE FROM session_turns WHERE session_id IN (SELECT id FROM sessions_to_prune)",
            [],
        )
        .map_err(|source| sqlite_error(path, source))?;
    let timeline_items_pruned = conn
        .execute(
            "DELETE FROM session_timeline_items WHERE session_id IN (SELECT id FROM sessions_to_prune)",
            [],
        )
        .map_err(|source| sqlite_error(path, source))?;
    let approvals_pruned = conn
        .execute(
            "DELETE FROM approvals WHERE session_id IN (SELECT id FROM sessions_to_prune)",
            [],
        )
        .map_err(|source| sqlite_error(path, source))?;
    let agent_events_pruned = conn
        .execute(
            "DELETE FROM agent_events WHERE session_id IN (SELECT id FROM sessions_to_prune)",
            [],
        )
        .map_err(|source| sqlite_error(path, source))?;
    let entries_pruned = conn
        .execute(
            "DELETE FROM session_entries WHERE session_id IN (SELECT id FROM sessions_to_prune)",
            [],
        )
        .map_err(|source| sqlite_error(path, source))?;
    conn.execute(
        "DELETE FROM sessions WHERE id IN (SELECT id FROM sessions_to_prune)",
        [],
    )
    .map_err(|source| sqlite_error(path, source))?;
    conn.execute_batch("DELETE FROM sessions_to_prune")
        .map_err(|source| sqlite_error(path, source))?;
    Ok(SqlitePruneReport {
        ended_before,
        sessions_pruned,
        entries_pruned,
        agent_events_pruned,
        approvals_pruned,
        timeline_items_pruned,
        continuations_pruned,
        inputs_pruned,
        turns_pruned,
        search_index_rows_pruned: fts_rows + trigram_rows,
    })
}
pub(super) fn session_record(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
) -> Result<Option<SessionRecord>> {
    conn.query_row(
        r#"
        SELECT id, source_json, agent_id, workspace, parent_session_id,
               active_leaf_entry_id, started_at, ended_at
        FROM sessions
        WHERE id = ?1
        "#,
        params![session_id.as_str()],
        |row| {
            let source_json: String = row.get(1)?;
            let workspace: Option<String> = row.get(3)?;
            let parent_session_id: Option<String> = row.get(4)?;
            let active_leaf_entry_id: Option<String> = row.get(5)?;
            let started_at: String = row.get(6)?;
            let ended_at: Option<String> = row.get(7)?;
            Ok((
                row.get::<_, String>(0)?,
                source_json,
                row.get::<_, Option<String>>(2)?,
                workspace,
                parent_session_id,
                active_leaf_entry_id,
                started_at,
                ended_at,
            ))
        },
    )
    .optional()
    .map_err(|source| sqlite_error(path, source))?
    .map(
        |(
            id,
            source_json,
            agent_id,
            workspace,
            parent_session_id,
            active_leaf_entry_id,
            started_at,
            ended_at,
        )| {
            Ok(SessionRecord {
                session_id: SessionId::from(id),
                source: serde_json::from_str(&source_json)?,
                agent_id,
                workspace: workspace.map(PathBuf::from),
                parent_session_id: parent_session_id.map(SessionId::from),
                active_leaf_entry_id: active_leaf_entry_id.map(SessionEntryId::from),
                started_at: parse_time(&started_at)?,
                ended_at: ended_at.as_deref().map(parse_time).transpose()?,
            })
        },
    )
    .transpose()
}

pub(super) fn session_records(conn: &Connection, path: &Path) -> Result<Vec<SessionRecord>> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, source_json, agent_id, workspace, parent_session_id,
                   active_leaf_entry_id, started_at, ended_at
            FROM sessions
            ORDER BY started_at ASC, id ASC
            "#,
        )
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map([], |row| {
            let source_json: String = row.get(1)?;
            let workspace: Option<String> = row.get(3)?;
            let parent_session_id: Option<String> = row.get(4)?;
            let active_leaf_entry_id: Option<String> = row.get(5)?;
            let started_at: String = row.get(6)?;
            let ended_at: Option<String> = row.get(7)?;
            Ok((
                row.get::<_, String>(0)?,
                source_json,
                row.get::<_, Option<String>>(2)?,
                workspace,
                parent_session_id,
                active_leaf_entry_id,
                started_at,
                ended_at,
            ))
        })
        .map_err(|source| sqlite_error(path, source))?;

    let mut records = Vec::new();
    for row in rows {
        let (
            id,
            source_json,
            agent_id,
            workspace,
            parent_session_id,
            active_leaf_entry_id,
            started_at,
            ended_at,
        ) = row.map_err(|source| sqlite_error(path, source))?;
        records.push(SessionRecord {
            session_id: SessionId::from(id),
            source: serde_json::from_str(&source_json)?,
            agent_id,
            workspace: workspace.map(PathBuf::from),
            parent_session_id: parent_session_id.map(SessionId::from),
            active_leaf_entry_id: active_leaf_entry_id.map(SessionEntryId::from),
            started_at: parse_time(&started_at)?,
            ended_at: ended_at.as_deref().map(parse_time).transpose()?,
        });
    }
    Ok(records)
}
