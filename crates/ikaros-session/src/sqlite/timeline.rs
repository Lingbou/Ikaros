// SPDX-License-Identifier: GPL-3.0-only

use super::*;

const TIMELINE_ENTRY: &str = "entry";
const TIMELINE_AGENT_EVENT: &str = "agent_event";
const TIMELINE_APPROVAL: &str = "approval";

pub(super) fn record_entry_timeline_item(
    conn: &Connection,
    path: &Path,
    entry: &SessionEntry,
) -> Result<()> {
    insert_timeline_item(
        conn,
        path,
        TimelineItemInput {
            session_id: &entry.session_id,
            turn_id: entry.turn_id.as_ref(),
            at: entry.at,
            item_kind: TIMELINE_ENTRY,
            item_id: entry.entry_id.as_str(),
        },
    )
}

pub(super) fn record_agent_event_timeline_item(
    conn: &Connection,
    path: &Path,
    event: &AgentEvent,
) -> Result<()> {
    insert_timeline_item(
        conn,
        path,
        TimelineItemInput {
            session_id: &event.session_id,
            turn_id: Some(&event.turn_id),
            at: event.at,
            item_kind: TIMELINE_AGENT_EVENT,
            item_id: event.event_id.as_str(),
        },
    )
}

pub(super) fn record_approval_timeline_item(
    conn: &Connection,
    path: &Path,
    approval: &ApprovalRecord,
) -> Result<()> {
    insert_timeline_item(
        conn,
        path,
        TimelineItemInput {
            session_id: &approval.session_id,
            turn_id: approval.turn_id.as_ref(),
            at: approval.at,
            item_kind: TIMELINE_APPROVAL,
            item_id: approval.approval_id.as_str(),
        },
    )
}

struct TimelineItemInput<'a> {
    session_id: &'a SessionId,
    turn_id: Option<&'a TurnId>,
    at: OffsetDateTime,
    item_kind: &'static str,
    item_id: &'a str,
}

fn insert_timeline_item(
    conn: &Connection,
    path: &Path,
    input: TimelineItemInput<'_>,
) -> Result<()> {
    conn.execute(
        r#"
        INSERT INTO session_timeline_items (
            session_id, turn_id, at, item_kind, item_id
        )
        VALUES (?1, ?2, ?3, ?4, ?5)
        ON CONFLICT(item_kind, item_id) DO UPDATE SET
            session_id = excluded.session_id,
            turn_id = excluded.turn_id,
            at = excluded.at
        "#,
        params![
            input.session_id.as_str(),
            input.turn_id.map(TurnId::as_str),
            format_time(input.at)?,
            input.item_kind,
            input.item_id,
        ],
    )
    .map_err(|source| sqlite_error(path, source))?;
    Ok(())
}

pub(super) fn session_timeline(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
) -> Result<Vec<SessionTimelineItem>> {
    load_timeline_items(conn, path, session_id, None, None, None)
}

pub(super) fn session_timeline_page(
    conn: &Connection,
    path: &Path,
    query: &SessionTimelineQuery,
) -> Result<SessionTimelinePage> {
    let page = query.page.max(1);
    let page_size = query.page_size.max(1);
    let offset = page.saturating_sub(1).saturating_mul(page_size);
    let total_items = count_timeline_items(conn, path, &query.session_id, query.turn_id.as_ref())?;
    let items = load_timeline_items(
        conn,
        path,
        &query.session_id,
        query.turn_id.as_ref(),
        Some(offset),
        Some(page_size),
    )?;
    Ok(SessionTimelinePage {
        session_id: query.session_id.clone(),
        turn_id: query.turn_id.clone(),
        page,
        page_size,
        total_items,
        items,
    })
}

fn count_timeline_items(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
    turn_id: Option<&TurnId>,
) -> Result<usize> {
    let count = if let Some(turn_id) = turn_id {
        conn.query_row(
            r#"
            SELECT COUNT(*)
            FROM session_timeline_items
            WHERE session_id = ?1
              AND turn_id = ?2
            "#,
            params![session_id.as_str(), turn_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
    } else {
        conn.query_row(
            r#"
            SELECT COUNT(*)
            FROM session_timeline_items
            WHERE session_id = ?1
            "#,
            params![session_id.as_str()],
            |row| row.get::<_, i64>(0),
        )
    }
    .map_err(|source| sqlite_error(path, source))?;
    Ok(count.max(0) as usize)
}

fn load_timeline_items(
    conn: &Connection,
    path: &Path,
    session_id: &SessionId,
    turn_id: Option<&TurnId>,
    offset: Option<usize>,
    limit: Option<usize>,
) -> Result<Vec<SessionTimelineItem>> {
    let paged = offset.is_some() || limit.is_some();
    let sql = match (turn_id.is_some(), paged) {
        (true, true) => {
            r#"
            SELECT sequence, item_kind, item_id
            FROM session_timeline_items
            WHERE session_id = ?1
              AND turn_id = ?2
            ORDER BY sequence ASC
            LIMIT ?3 OFFSET ?4
            "#
        }
        (true, false) => {
            r#"
            SELECT sequence, item_kind, item_id
            FROM session_timeline_items
            WHERE session_id = ?1
              AND turn_id = ?2
            ORDER BY sequence ASC
            "#
        }
        (false, true) => {
            r#"
            SELECT sequence, item_kind, item_id
            FROM session_timeline_items
            WHERE session_id = ?1
            ORDER BY sequence ASC
            LIMIT ?2 OFFSET ?3
            "#
        }
        (false, false) => {
            r#"
            SELECT sequence, item_kind, item_id
            FROM session_timeline_items
            WHERE session_id = ?1
            ORDER BY sequence ASC
            "#
        }
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|source| sqlite_error(path, source))?;
    let row_mapper = |row: &rusqlite::Row<'_>| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    };
    let mut rows = match (turn_id, paged) {
        (Some(turn_id), true) => stmt
            .query_map(
                params![
                    session_id.as_str(),
                    turn_id.as_str(),
                    sqlite_limit(limit.unwrap_or(usize::MAX)),
                    sqlite_offset(offset.unwrap_or(0))
                ],
                row_mapper,
            )
            .map_err(|source| sqlite_error(path, source))?,
        (Some(turn_id), false) => stmt
            .query_map(params![session_id.as_str(), turn_id.as_str()], row_mapper)
            .map_err(|source| sqlite_error(path, source))?,
        (None, true) => stmt
            .query_map(
                params![
                    session_id.as_str(),
                    sqlite_limit(limit.unwrap_or(usize::MAX)),
                    sqlite_offset(offset.unwrap_or(0))
                ],
                row_mapper,
            )
            .map_err(|source| sqlite_error(path, source))?,
        (None, false) => stmt
            .query_map(params![session_id.as_str()], row_mapper)
            .map_err(|source| sqlite_error(path, source))?,
    };
    let mut items = Vec::new();
    for row in &mut rows {
        let (sequence, item_kind, item_id) = row.map_err(|source| sqlite_error(path, source))?;
        let record = match item_kind.as_str() {
            TIMELINE_ENTRY => {
                let entry_id = SessionEntryId::from(item_id);
                let entry = session_entry(conn, path, &entry_id)?.ok_or_else(|| {
                    IkarosError::Message(format!("session timeline entry missing: {entry_id}"))
                })?;
                SessionTimelineRecord::Entry(entry)
            }
            TIMELINE_AGENT_EVENT => {
                let event = agent_event(conn, path, &item_id)?.ok_or_else(|| {
                    IkarosError::Message(format!("session timeline event missing: {item_id}"))
                })?;
                SessionTimelineRecord::AgentEvent(event)
            }
            TIMELINE_APPROVAL => {
                let approval = approval_record(conn, path, &item_id)?.ok_or_else(|| {
                    IkarosError::Message(format!("session timeline approval missing: {item_id}"))
                })?;
                SessionTimelineRecord::Approval(approval)
            }
            _ => {
                return Err(IkarosError::Message(format!(
                    "unknown session timeline item kind: {item_kind}"
                )));
            }
        };
        items.push(SessionTimelineItem {
            sequence,
            at: record.at(),
            turn_id: record.turn_id(),
            record,
        });
    }
    Ok(items)
}
