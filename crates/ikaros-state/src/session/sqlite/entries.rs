// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) fn append_entry(conn: &Connection, path: &Path, entry: &SessionEntry) -> Result<()> {
    insert_missing_session(conn, path, &entry.session_id)?;
    let payload_json = serde_json::to_string(&entry.payload)?;
    conn.execute(
        r#"
        INSERT INTO session_entries (
            id, session_id, parent_entry_id, turn_id, at, kind, visible_text, payload_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            entry.entry_id.as_str(),
            entry.session_id.as_str(),
            entry.parent_entry_id.as_ref().map(SessionEntryId::as_str),
            entry.turn_id.as_ref().map(TurnId::as_str),
            format_time(entry.at)?,
            entry_kind_to_str(entry.kind),
            entry.visible_text.as_deref(),
            payload_json,
        ],
    )
    .map_err(|source| sqlite_error(path, source))?;
    record_entry_timeline_item(conn, path, entry)?;
    conn.execute(
        "UPDATE sessions SET active_leaf_entry_id = ?1 WHERE id = ?2",
        params![entry.entry_id.as_str(), entry.session_id.as_str()],
    )
    .map_err(|source| sqlite_error(path, source))?;
    index_session_entry(conn, path, entry)?;
    Ok(())
}

pub(super) fn index_session_entry(
    conn: &Connection,
    path: &Path,
    entry: &SessionEntry,
) -> Result<()> {
    let Some(text) = entry.visible_text.as_deref() else {
        return Ok(());
    };
    if text.trim().is_empty() {
        return Ok(());
    }
    let turn_id = entry.turn_id.as_ref().map(TurnId::as_str);
    for table in ["session_entries_fts", "session_entries_trigram"] {
        conn.execute(
            &format!(
                r#"
                INSERT INTO {table} (entry_id, session_id, turn_id, kind, visible_text)
                VALUES (?1, ?2, ?3, ?4, ?5)
                "#
            ),
            params![
                entry.entry_id.as_str(),
                entry.session_id.as_str(),
                turn_id,
                entry_kind_to_str(entry.kind),
                text,
            ],
        )
        .map_err(|source| sqlite_error(path, source))?;
    }
    Ok(())
}

pub(super) fn rebuild_missing_entry_search_indexes(conn: &Connection, path: &Path) -> Result<()> {
    for table in ["session_entries_fts", "session_entries_trigram"] {
        conn.execute(
            &format!(
                r#"
                INSERT INTO {table} (entry_id, session_id, turn_id, kind, visible_text)
                SELECT e.id, e.session_id, e.turn_id, e.kind, e.visible_text
                FROM session_entries e
                WHERE e.visible_text IS NOT NULL
                  AND trim(e.visible_text) != ''
                  AND NOT EXISTS (
                    SELECT 1 FROM {table} idx
                    WHERE idx.entry_id = e.id
                  )
                "#
            ),
            [],
        )
        .map_err(|source| sqlite_error(path, source))?;
    }
    Ok(())
}
pub(super) fn session_entry(
    conn: &Connection,
    path: &Path,
    entry_id: &SessionEntryId,
) -> Result<Option<SessionEntry>> {
    conn.query_row(
        r#"
        SELECT id, session_id, parent_entry_id, turn_id, at, kind, visible_text, payload_json
        FROM session_entries
        WHERE id = ?1
        "#,
        params![entry_id.as_str()],
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
    .optional()
    .map_err(|source| sqlite_error(path, source))?
    .map(session_entry_from_parts)
    .transpose()
}

pub(super) fn active_branch(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
) -> Result<Option<SessionBranch>> {
    let Some(session) = session_record(conn, path, session_id)? else {
        return Ok(None);
    };
    let Some(mut current_id) = session.active_leaf_entry_id.clone() else {
        return Ok(Some(SessionBranch {
            session,
            entries: Vec::new(),
        }));
    };
    let mut seen = HashSet::new();
    let mut entries = Vec::new();
    loop {
        if !seen.insert(current_id.as_str().to_owned()) {
            return Err(IkarosError::Message(format!(
                "session {} has a cycle at entry {}",
                session.session_id, current_id
            )));
        }
        let Some(entry) = session_entry(conn, path, &current_id)? else {
            return Err(IkarosError::Message(format!(
                "session {} active branch points at missing entry {}",
                session.session_id, current_id
            )));
        };
        if entry.session_id != session.session_id {
            return Err(IkarosError::Message(format!(
                "session {} active branch crossed into session {} at entry {}",
                session.session_id, entry.session_id, entry.entry_id
            )));
        }
        let parent_entry_id = entry.parent_entry_id.clone();
        entries.push(entry);
        let Some(parent_entry_id) = parent_entry_id else {
            break;
        };
        current_id = parent_entry_id;
    }
    entries.reverse();
    Ok(Some(SessionBranch { session, entries }))
}

pub(super) fn set_active_leaf(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
    entry_id: &SessionEntryId,
) -> Result<()> {
    let Some(entry) = session_entry(conn, path, entry_id)? else {
        return Err(IkarosError::Message(format!(
            "session entry not found: {entry_id}"
        )));
    };
    if &entry.session_id != session_id {
        return Err(IkarosError::Message(format!(
            "entry {entry_id} belongs to session {}, not {session_id}",
            entry.session_id
        )));
    }
    let updated = conn
        .execute(
            "UPDATE sessions SET active_leaf_entry_id = ?1 WHERE id = ?2",
            params![entry_id.as_str(), session_id.as_str()],
        )
        .map_err(|source| sqlite_error(path, source))?;
    if updated == 0 {
        return Err(IkarosError::Message(format!(
            "session not found while setting active leaf: {session_id}"
        )));
    }
    Ok(())
}

pub(super) fn ensure_parent_entry(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
    parent_entry_id: &SessionEntryId,
) -> Result<()> {
    let Some(parent) = session_entry(conn, path, parent_entry_id)? else {
        return Err(IkarosError::Message(format!(
            "parent session entry not found: {parent_entry_id}"
        )));
    };
    if &parent.session_id != session_id {
        return Err(IkarosError::Message(format!(
            "parent entry {parent_entry_id} belongs to session {}, not {session_id}",
            parent.session_id
        )));
    }
    Ok(())
}

pub(super) fn append_branch_summary(
    conn: &Connection,
    path: &Path,
    input: &SessionBranchSummaryInput,
) -> Result<SessionEntry> {
    ensure_parent_entry(conn, path, &input.session_id, &input.parent_entry_id)?;
    let mut entry = SessionEntry::new(input.session_id.clone(), SessionEntryKind::BranchSummary);
    entry.parent_entry_id = Some(input.parent_entry_id.clone());
    entry.visible_text = Some(input.summary.clone());
    entry.payload = serde_json::json!({
        "operation": "branch_summary",
        "parent_entry_id": input.parent_entry_id.as_str(),
        "summary": &input.summary,
        "data": &input.payload,
    });
    append_entry(conn, path, &entry)?;
    Ok(entry)
}

pub(super) fn append_compaction(
    conn: &Connection,
    path: &Path,
    input: &SessionCompactionInput,
) -> Result<SessionEntry> {
    ensure_parent_entry(conn, path, &input.session_id, &input.parent_entry_id)?;
    let compacted_entry_ids = input
        .compacted_entry_ids
        .iter()
        .map(SessionEntryId::as_str)
        .collect::<Vec<_>>();
    let mut entry = SessionEntry::new(input.session_id.clone(), SessionEntryKind::Compaction);
    entry.parent_entry_id = Some(input.parent_entry_id.clone());
    entry.visible_text = Some(input.summary.clone());
    entry.payload = serde_json::json!({
        "operation": "compaction",
        "parent_entry_id": input.parent_entry_id.as_str(),
        "compacted_entry_ids": compacted_entry_ids,
        "summary": &input.summary,
        "data": &input.payload,
    });
    append_entry(conn, path, &entry)?;
    Ok(entry)
}

pub(super) fn append_retry_marker(
    conn: &Connection,
    path: &Path,
    input: &SessionRetryInput,
) -> Result<SessionEntry> {
    ensure_parent_entry(conn, path, &input.session_id, &input.parent_entry_id)?;
    let mut entry = SessionEntry::new(input.session_id.clone(), SessionEntryKind::Leaf);
    entry.parent_entry_id = Some(input.parent_entry_id.clone());
    entry.visible_text = input.reason.clone();
    entry.payload = serde_json::json!({
        "operation": "retry",
        "parent_entry_id": input.parent_entry_id.as_str(),
        "reason": &input.reason,
        "data": &input.payload,
    });
    append_entry(conn, path, &entry)?;
    Ok(entry)
}
