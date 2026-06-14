// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AgentEvent, AgentEventKind, ApprovalRecord, ApprovalStatus, SessionBranch,
    SessionBranchSummaryInput, SessionCompactionInput, SessionEntry, SessionEntryId,
    SessionEntryKind, SessionId, SessionRecord, SessionRetryInput, SessionSearchHit,
    SessionSearchIndex, SessionSearchQuery, SessionSource, SessionStore, SessionWriter, TurnId,
};
use ikaros_core::{IkarosError, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const SESSION_SCHEMA_VERSION: i64 = 2;

#[derive(Debug, Clone)]
pub struct SqliteSessionStore {
    path: PathBuf,
}

impl SqliteSessionStore {
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            path: state_dir.into().join("state.db"),
        }
    }

    pub fn from_file(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn open(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let conn =
            Connection::open(&self.path).map_err(|source| sqlite_error(&self.path, source))?;
        conn.busy_timeout(Duration::from_secs(5))
            .map_err(|source| sqlite_error(&self.path, source))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|source| sqlite_error(&self.path, source))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|source| sqlite_error(&self.path, source))?;
        self.ensure_schema(&conn)?;
        Ok(conn)
    }

    fn ensure_schema(&self, conn: &Connection) -> Result<()> {
        let version = conn
            .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
            .map_err(|source| sqlite_error(&self.path, source))?;
        if version > SESSION_SCHEMA_VERSION {
            return Err(IkarosError::Message(format!(
                "state.db schema version {version} is newer than supported version {SESSION_SCHEMA_VERSION}"
            )));
        }
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                source_json TEXT NOT NULL,
                agent_id TEXT,
                workspace TEXT,
                parent_session_id TEXT,
                active_leaf_entry_id TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT
            );

            CREATE TABLE IF NOT EXISTS session_entries (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                parent_entry_id TEXT,
                turn_id TEXT,
                at TEXT NOT NULL,
                kind TEXT NOT NULL,
                visible_text TEXT,
                payload_json TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            );
            CREATE INDEX IF NOT EXISTS session_entries_session_at_idx
                ON session_entries(session_id, at);
            CREATE INDEX IF NOT EXISTS session_entries_parent_idx
                ON session_entries(parent_entry_id);

            CREATE VIRTUAL TABLE IF NOT EXISTS session_entries_fts USING fts5(
                entry_id UNINDEXED,
                session_id UNINDEXED,
                turn_id UNINDEXED,
                kind UNINDEXED,
                visible_text,
                tokenize = 'unicode61'
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS session_entries_trigram USING fts5(
                entry_id UNINDEXED,
                session_id UNINDEXED,
                turn_id UNINDEXED,
                kind UNINDEXED,
                visible_text,
                tokenize = 'trigram'
            );

            CREATE TABLE IF NOT EXISTS agent_events (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                parent_event_id TEXT,
                at TEXT NOT NULL,
                source TEXT NOT NULL,
                kind_json TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            );
            CREATE INDEX IF NOT EXISTS agent_events_session_at_idx
                ON agent_events(session_id, at);
            CREATE INDEX IF NOT EXISTS agent_events_turn_idx
                ON agent_events(turn_id);

            CREATE TABLE IF NOT EXISTS approvals (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                turn_id TEXT,
                at TEXT NOT NULL,
                status TEXT NOT NULL,
                request_json TEXT NOT NULL,
                decision_json TEXT,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            );
            CREATE INDEX IF NOT EXISTS approvals_session_at_idx
                ON approvals(session_id, at);

            PRAGMA user_version = 2;
            "#,
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        rebuild_missing_entry_search_indexes(conn, &self.path)
    }
}

impl SessionStore for SqliteSessionStore {
    fn upsert_session(&self, session: &SessionRecord) -> Result<()> {
        let conn = self.open()?;
        upsert_session(&conn, &self.path, session)
    }

    fn finish_session(&self, session_id: &SessionId, ended_at: OffsetDateTime) -> Result<()> {
        let conn = self.open()?;
        finish_session(&conn, &self.path, session_id, ended_at)
    }

    fn begin_turn(
        &self,
        session: &SessionRecord,
        turn_id: &TurnId,
    ) -> Result<Box<dyn SessionWriter>> {
        let conn = self.open()?;
        conn.execute_batch("BEGIN IMMEDIATE TRANSACTION")
            .map_err(|source| sqlite_error(&self.path, source))?;
        upsert_session(&conn, &self.path, session)?;
        Ok(Box::new(SqliteSessionWriter {
            conn: Some(conn),
            path: self.path.clone(),
            session_id: session.session_id.clone(),
            turn_id: turn_id.clone(),
            failed: false,
        }))
    }

    fn append_entry(&self, entry: &SessionEntry) -> Result<()> {
        let conn = self.open()?;
        append_entry(&conn, &self.path, entry)
    }

    fn append_agent_event(&self, event: &AgentEvent) -> Result<()> {
        let conn = self.open()?;
        append_agent_event(&conn, &self.path, event)
    }

    fn append_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        let conn = self.open()?;
        append_approval(&conn, &self.path, approval)
    }

    fn get_session(&self, session_id: &SessionId) -> Result<Option<SessionRecord>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let conn = self.open()?;
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
        .map_err(|source| sqlite_error(&self.path, source))?
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

    fn session_entry(&self, entry_id: &SessionEntryId) -> Result<Option<SessionEntry>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let conn = self.open()?;
        session_entry(&conn, &self.path, entry_id)
    }

    fn session_entries(&self, session_id: &SessionId) -> Result<Vec<SessionEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, session_id, parent_entry_id, turn_id, at, kind, visible_text, payload_json
                FROM session_entries
                WHERE session_id = ?1
                ORDER BY at ASC, rowid ASC
                "#,
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
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
                    row.get::<_, String>(7)?,
                ))
            })
            .map_err(|source| sqlite_error(&self.path, source))?;
        let mut entries = Vec::new();
        for row in rows {
            let row = row.map_err(|source| sqlite_error(&self.path, source))?;
            entries.push(session_entry_from_parts(row)?);
        }
        Ok(entries)
    }

    fn active_branch(&self, session_id: &SessionId) -> Result<Option<SessionBranch>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let conn = self.open()?;
        active_branch(&conn, &self.path, session_id)
    }

    fn set_active_leaf(&self, session_id: &SessionId, entry_id: &SessionEntryId) -> Result<()> {
        let conn = self.open()?;
        set_active_leaf(&conn, &self.path, session_id, entry_id)
    }

    fn append_branch_summary(&self, input: &SessionBranchSummaryInput) -> Result<SessionEntry> {
        let conn = self.open()?;
        append_branch_summary(&conn, &self.path, input)
    }

    fn append_compaction(&self, input: &SessionCompactionInput) -> Result<SessionEntry> {
        let conn = self.open()?;
        append_compaction(&conn, &self.path, input)
    }

    fn append_retry_marker(&self, input: &SessionRetryInput) -> Result<SessionEntry> {
        let conn = self.open()?;
        append_retry_marker(&conn, &self.path, input)
    }

    fn search_entries(&self, query: &SessionSearchQuery) -> Result<Vec<SessionSearchHit>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let query_text = query.query.trim();
        if query_text.is_empty() || query.limit == 0 {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        search_entries(&conn, &self.path, query)
    }

    fn agent_events(&self, session_id: &SessionId) -> Result<Vec<AgentEvent>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, session_id, turn_id, parent_event_id, at, source, kind_json, payload_json
                FROM agent_events
                WHERE session_id = ?1
                ORDER BY at ASC, rowid ASC
                "#,
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
        let rows = stmt
            .query_map(params![session_id.as_str()], |row| {
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
            })
            .map_err(|source| sqlite_error(&self.path, source))?;
        let mut events = Vec::new();
        for row in rows {
            let (id, session_id, turn_id, parent_event_id, at, source, kind_json, payload_json) =
                row.map_err(|source| sqlite_error(&self.path, source))?;
            events.push(AgentEvent {
                event_id: id.into(),
                session_id: session_id.into(),
                turn_id: turn_id.into(),
                parent_event_id: parent_event_id.map(Into::into),
                at: parse_time(&at)?,
                source: event_source_from_str(&source)?,
                kind: serde_json::from_str::<AgentEventKind>(&kind_json)?,
                payload: serde_json::from_str(&payload_json)?,
            });
        }
        Ok(events)
    }

    fn approval_record(&self, approval_id: &str) -> Result<Option<ApprovalRecord>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let conn = self.open()?;
        approval_record(&conn, &self.path, approval_id)
    }

    fn approvals(&self, session_id: &SessionId) -> Result<Vec<ApprovalRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        let mut stmt = conn
            .prepare(
                r#"
                SELECT id, session_id, turn_id, at, status, request_json, decision_json
                FROM approvals
                WHERE session_id = ?1
                ORDER BY at ASC, rowid ASC
                "#,
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
        let rows = stmt
            .query_map(params![session_id.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                ))
            })
            .map_err(|source| sqlite_error(&self.path, source))?;
        let mut approvals = Vec::new();
        for row in rows {
            approvals.push(approval_record_from_parts(
                row.map_err(|source| sqlite_error(&self.path, source))?,
            )?);
        }
        Ok(approvals)
    }
}

type ApprovalRecordRow = (
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    Option<String>,
);

fn approval_record_from_parts(row: ApprovalRecordRow) -> Result<ApprovalRecord> {
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

struct SqliteSessionWriter {
    conn: Option<Connection>,
    path: PathBuf,
    session_id: SessionId,
    turn_id: TurnId,
    failed: bool,
}

impl SqliteSessionWriter {
    fn conn(&self) -> Result<&Connection> {
        self.conn.as_ref().ok_or_else(|| {
            IkarosError::Message("session writer transaction is already closed".into())
        })
    }

    fn mark<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.is_err() {
            self.failed = true;
        }
        result
    }

    fn ensure_session_scope(&mut self, session_id: &SessionId) -> Result<()> {
        if session_id != &self.session_id {
            self.failed = true;
            return Err(IkarosError::Message(format!(
                "session writer expected session {}, got {}",
                self.session_id, session_id
            )));
        }
        Ok(())
    }

    fn ensure_optional_turn_scope(&mut self, turn_id: Option<&TurnId>) -> Result<()> {
        if let Some(turn_id) = turn_id {
            if turn_id != &self.turn_id {
                self.failed = true;
                return Err(IkarosError::Message(format!(
                    "session writer expected turn {}, got {}",
                    self.turn_id, turn_id
                )));
            }
        }
        Ok(())
    }

    fn ensure_turn_scope(&mut self, turn_id: &TurnId) -> Result<()> {
        if turn_id != &self.turn_id {
            self.failed = true;
            return Err(IkarosError::Message(format!(
                "session writer expected turn {}, got {}",
                self.turn_id, turn_id
            )));
        }
        Ok(())
    }
}

impl SessionWriter for SqliteSessionWriter {
    fn append_entry(&mut self, entry: &SessionEntry) -> Result<()> {
        self.ensure_session_scope(&entry.session_id)?;
        self.ensure_optional_turn_scope(entry.turn_id.as_ref())?;
        let result = append_entry(self.conn()?, &self.path, entry);
        self.mark(result)
    }

    fn append_agent_event(&mut self, event: &AgentEvent) -> Result<()> {
        self.ensure_session_scope(&event.session_id)?;
        self.ensure_turn_scope(&event.turn_id)?;
        let result = append_agent_event(self.conn()?, &self.path, event);
        self.mark(result)
    }

    fn append_approval(&mut self, approval: &ApprovalRecord) -> Result<()> {
        self.ensure_session_scope(&approval.session_id)?;
        self.ensure_optional_turn_scope(approval.turn_id.as_ref())?;
        let result = append_approval(self.conn()?, &self.path, approval);
        self.mark(result)
    }

    fn commit(mut self: Box<Self>) -> Result<()> {
        let Some(conn) = self.conn.take() else {
            return Err(IkarosError::Message(
                "session writer transaction is already closed".into(),
            ));
        };
        if self.failed {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(IkarosError::Message(
                "session writer transaction has failed and was rolled back".into(),
            ));
        }
        match conn.execute_batch("COMMIT") {
            Ok(()) => Ok(()),
            Err(source) => {
                let error = sqlite_error(&self.path, source);
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }

    fn rollback(mut self: Box<Self>) -> Result<()> {
        let Some(conn) = self.conn.take() else {
            return Ok(());
        };
        conn.execute_batch("ROLLBACK")
            .map_err(|source| sqlite_error(&self.path, source))
    }
}

impl Drop for SqliteSessionWriter {
    fn drop(&mut self) {
        if let Some(conn) = self.conn.take() {
            let _ = conn.execute_batch("ROLLBACK");
        }
    }
}

fn upsert_session(conn: &Connection, path: &Path, session: &SessionRecord) -> Result<()> {
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

fn insert_missing_session(conn: &Connection, path: &Path, session_id: &SessionId) -> Result<()> {
    let started_at = format_time(OffsetDateTime::now_utc())?;
    let source_json = serde_json::to_string(&SessionSource::Runtime)?;
    conn.execute(
        "INSERT OR IGNORE INTO sessions (id, source_json, started_at) VALUES (?1, ?2, ?3)",
        params![session_id.as_str(), source_json, started_at],
    )
    .map_err(|source| sqlite_error(path, source))?;
    Ok(())
}

fn finish_session(
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

fn append_entry(conn: &Connection, path: &Path, entry: &SessionEntry) -> Result<()> {
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
    conn.execute(
        "UPDATE sessions SET active_leaf_entry_id = ?1 WHERE id = ?2",
        params![entry.entry_id.as_str(), entry.session_id.as_str()],
    )
    .map_err(|source| sqlite_error(path, source))?;
    index_session_entry(conn, path, entry)?;
    Ok(())
}

fn index_session_entry(conn: &Connection, path: &Path, entry: &SessionEntry) -> Result<()> {
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

fn rebuild_missing_entry_search_indexes(conn: &Connection, path: &Path) -> Result<()> {
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

fn append_agent_event(conn: &Connection, path: &Path, event: &AgentEvent) -> Result<()> {
    insert_missing_session(conn, path, &event.session_id)?;
    conn.execute(
        r#"
        INSERT INTO agent_events (
            id, session_id, turn_id, parent_event_id, at, source, kind_json, payload_json
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
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
    Ok(())
}

fn append_approval(conn: &Connection, path: &Path, approval: &ApprovalRecord) -> Result<()> {
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
    Ok(())
}

fn approval_record(
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

fn session_record(
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

type SessionEntryRow = (
    String,
    String,
    Option<String>,
    Option<String>,
    String,
    String,
    Option<String>,
    String,
);

fn session_entry_from_parts(row: SessionEntryRow) -> Result<SessionEntry> {
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

fn session_entry(
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

fn active_branch(
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

fn set_active_leaf(
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

fn ensure_parent_entry(
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

fn append_branch_summary(
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

fn append_compaction(
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

fn append_retry_marker(
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

fn search_entries(
    conn: &Connection,
    path: &Path,
    query: &SessionSearchQuery,
) -> Result<Vec<SessionSearchHit>> {
    let query_text = query.query.trim();
    if query_text.is_empty() || query.limit == 0 {
        return Ok(Vec::new());
    }

    let mut hits = Vec::new();
    let mut seen = HashSet::new();
    let fts_query = quoted_fts_query(query_text);
    collect_index_hits(
        conn,
        path,
        &mut hits,
        &mut seen,
        ("session_entries_fts", SessionSearchIndex::Fts),
        &fts_query,
        query,
    )?;
    collect_index_hits(
        conn,
        path,
        &mut hits,
        &mut seen,
        ("session_entries_trigram", SessionSearchIndex::Trigram),
        &fts_query,
        query,
    )?;
    collect_substring_hits(conn, path, &mut hits, &mut seen, query)?;

    hits.sort_by(|left, right| {
        left.score
            .partial_cmp(&right.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.entry.at.cmp(&right.entry.at))
            .then_with(|| left.entry.entry_id.cmp(&right.entry.entry_id))
    });
    hits.truncate(query.limit);
    Ok(hits)
}

fn collect_index_hits(
    conn: &Connection,
    path: &Path,
    hits: &mut Vec<SessionSearchHit>,
    seen: &mut HashSet<String>,
    index_spec: (&str, SessionSearchIndex),
    fts_query: &str,
    query: &SessionSearchQuery,
) -> Result<()> {
    let (table, index) = index_spec;
    let sql = format!(
        r#"
        SELECT e.id, e.session_id, e.parent_entry_id, e.turn_id, e.at, e.kind, e.visible_text,
               e.payload_json, bm25({table}) AS score
        FROM {table}
        JOIN session_entries e ON e.id = {table}.entry_id
        WHERE {table} MATCH ?1
          AND (?2 IS NULL OR e.session_id = ?2)
        ORDER BY score ASC, e.at ASC
        LIMIT ?3
        "#
    );
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map(
            params![
                fts_query,
                query.session_id.as_ref().map(SessionId::as_str),
                query.limit as i64,
            ],
            |row| {
                Ok((
                    (
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, String>(7)?,
                    ),
                    row.get::<_, f64>(8)?,
                ))
            },
        )
        .map_err(|source| sqlite_error(path, source))?;
    for row in rows {
        let (entry_row, score) = row.map_err(|source| sqlite_error(path, source))?;
        let entry = session_entry_from_parts(entry_row)?;
        if seen.insert(entry.entry_id.as_str().to_owned()) {
            hits.push(SessionSearchHit {
                snippet: entry_snippet(entry.visible_text.as_deref(), &query.query),
                entry,
                score,
                index,
            });
        }
    }
    Ok(())
}

fn collect_substring_hits(
    conn: &Connection,
    path: &Path,
    hits: &mut Vec<SessionSearchHit>,
    seen: &mut HashSet<String>,
    query: &SessionSearchQuery,
) -> Result<()> {
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, session_id, parent_entry_id, turn_id, at, kind, visible_text, payload_json
            FROM session_entries
            WHERE visible_text IS NOT NULL
              AND instr(visible_text, ?1) > 0
              AND (?2 IS NULL OR session_id = ?2)
            ORDER BY at ASC, rowid ASC
            LIMIT ?3
            "#,
        )
        .map_err(|source| sqlite_error(path, source))?;
    let rows = stmt
        .query_map(
            params![
                query.query.trim(),
                query.session_id.as_ref().map(SessionId::as_str),
                query.limit as i64,
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
    for row in rows {
        let entry = session_entry_from_parts(row.map_err(|source| sqlite_error(path, source))?)?;
        if seen.insert(entry.entry_id.as_str().to_owned()) {
            hits.push(SessionSearchHit {
                snippet: entry_snippet(entry.visible_text.as_deref(), &query.query),
                score: 10_000.0 + hits.len() as f64,
                entry,
                index: SessionSearchIndex::Substring,
            });
        }
    }
    Ok(())
}

fn quoted_fts_query(query: &str) -> String {
    format!("\"{}\"", query.trim().replace('"', "\"\""))
}

fn entry_snippet(visible_text: Option<&str>, query: &str) -> String {
    let Some(text) = visible_text else {
        return String::new();
    };
    let query = query.trim();
    if query.is_empty() {
        return text.chars().take(160).collect();
    }
    let start_byte = text.find(query).unwrap_or(0);
    let prefix_start = text[..start_byte]
        .char_indices()
        .rev()
        .nth(40)
        .map(|(idx, _)| idx)
        .unwrap_or(0);
    let end_byte = text[start_byte..]
        .char_indices()
        .nth(query.chars().count().saturating_add(80))
        .map(|(idx, _)| start_byte + idx)
        .unwrap_or(text.len());
    let mut snippet = String::new();
    if prefix_start > 0 {
        snippet.push_str("...");
    }
    snippet.push_str(&text[prefix_start..end_byte]);
    if end_byte < text.len() {
        snippet.push_str("...");
    }
    snippet
}

fn format_time(value: OffsetDateTime) -> Result<String> {
    value.format(&Rfc3339).map_err(IkarosError::Time)
}

fn parse_time(value: &str) -> Result<OffsetDateTime> {
    OffsetDateTime::parse(value, &Rfc3339)
        .map_err(|source| IkarosError::Message(format!("invalid state.db timestamp: {source}")))
}

fn entry_kind_to_str(kind: SessionEntryKind) -> &'static str {
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

fn entry_kind_from_str(value: &str) -> Result<SessionEntryKind> {
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

fn event_source_to_str(source: crate::AgentEventSource) -> &'static str {
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

fn event_source_from_str(value: &str) -> Result<crate::AgentEventSource> {
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

fn approval_status_to_str(status: ApprovalStatus) -> &'static str {
    match status {
        ApprovalStatus::Requested => "requested",
        ApprovalStatus::Approved => "approved",
        ApprovalStatus::Denied => "denied",
        ApprovalStatus::Expired => "expired",
        ApprovalStatus::Executed => "executed",
    }
}

fn approval_status_from_str(value: &str) -> Result<ApprovalStatus> {
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

fn sqlite_error(path: &Path, source: rusqlite::Error) -> IkarosError {
    IkarosError::Message(format!("sqlite error at {}: {source}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, ApprovalStatus,
        PersistingAgentEventSink, PersistingAgentTurnSink,
    };
    use ikaros_models::ModelStreamEvent;
    use serde_json::json;
    use std::sync::Arc;

    fn sample_session(session_id: SessionId) -> SessionRecord {
        let mut session = SessionRecord::new(session_id, SessionSource::Cli);
        session.agent_id = Some("build".into());
        session
    }

    fn sample_entry(
        session_id: SessionId,
        turn_id: TurnId,
        kind: SessionEntryKind,
        text: &str,
    ) -> SessionEntry {
        let mut entry = SessionEntry::new(session_id, kind);
        entry.turn_id = Some(turn_id);
        entry.visible_text = Some(text.into());
        entry.payload = json!({ "text": text });
        entry
    }

    fn sample_events(session_id: SessionId, turn_id: TurnId) -> Vec<AgentEvent> {
        let start = AgentEvent::new(
            session_id.clone(),
            turn_id.clone(),
            None,
            AgentEventSource::Runtime,
            AgentEventKind::TurnStart,
            json!({"step": 1}),
        );
        let model = AgentEvent::new(
            session_id.clone(),
            turn_id.clone(),
            Some(start.event_id.clone()),
            AgentEventSource::Model,
            AgentEventKind::ModelStream(ModelStreamEvent::TextDelta("hello".into())),
            json!({"step": 2}),
        );
        let end = AgentEvent::new(
            session_id,
            turn_id,
            Some(model.event_id.clone()),
            AgentEventSource::Runtime,
            AgentEventKind::TurnEnd,
            json!({"step": 3}),
        );
        vec![start, model, end]
    }

    fn sample_approval(session_id: SessionId, turn_id: TurnId) -> ApprovalRecord {
        ApprovalRecord {
            approval_id: "approval-turn".into(),
            session_id,
            turn_id: Some(turn_id),
            at: OffsetDateTime::now_utc(),
            status: ApprovalStatus::Requested,
            request: json!({"tool": "write_file"}),
            decision: None,
        }
    }

    #[test]
    fn sqlite_store_replays_session_timeline() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("session-a");
        let mut session = SessionRecord::new(session_id.clone(), SessionSource::Cli);
        session.agent_id = Some("build".into());
        store.upsert_session(&session).expect("session");

        let mut user = SessionEntry::new(session_id.clone(), SessionEntryKind::UserMessage);
        user.visible_text = Some("hello".into());
        let parent_id = user.entry_id.clone();
        store.append_entry(&user).expect("user entry");

        let mut assistant =
            SessionEntry::new(session_id.clone(), SessionEntryKind::AssistantMessage);
        assistant.parent_entry_id = Some(parent_id);
        assistant.visible_text = Some("world".into());
        store.append_entry(&assistant).expect("assistant entry");

        let turn_id = TurnId::from("turn-a");
        let start = AgentEvent::new(
            session_id.clone(),
            turn_id.clone(),
            None,
            AgentEventSource::Runtime,
            AgentEventKind::TurnStart,
            json!({"iteration": 1}),
        );
        store.append_agent_event(&start).expect("start event");
        let model = AgentEvent::new(
            session_id.clone(),
            turn_id,
            Some(start.event_id.clone()),
            AgentEventSource::Model,
            AgentEventKind::ModelStream(ModelStreamEvent::TextDelta("world".into())),
            serde_json::Value::Null,
        );
        store.append_agent_event(&model).expect("model event");

        store
            .append_approval(&ApprovalRecord {
                approval_id: "approval-a".into(),
                session_id: session_id.clone(),
                turn_id: None,
                at: OffsetDateTime::now_utc(),
                status: ApprovalStatus::Requested,
                request: json!({"tool": "write_file"}),
                decision: None,
            })
            .expect("approval");

        let replay = store
            .replay_session(&session_id)
            .expect("replay")
            .expect("session exists");
        assert_eq!(replay.session.agent_id.as_deref(), Some("build"));
        assert_eq!(replay.entries.len(), 2);
        assert_eq!(
            replay.entries[1].parent_entry_id,
            Some(replay.entries[0].entry_id.clone())
        );
        assert_eq!(replay.agent_events.len(), 2);
        assert_eq!(
            replay.agent_events[1].parent_event_id,
            Some(replay.agent_events[0].event_id.clone())
        );
        assert_eq!(replay.approvals.len(), 1);
    }

    #[test]
    fn persisting_event_sink_creates_session_and_appends_events() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::new(temp.path()));
        let sink = PersistingAgentEventSink::new(store.clone())
            .with_source(SessionSource::Cli)
            .with_agent_id("build")
            .with_workspace(temp.path().join("workspace"));
        let event = AgentEvent::new(
            "session-b",
            "turn-b",
            None,
            AgentEventSource::Runtime,
            AgentEventKind::SessionStart,
            serde_json::Value::Null,
        );

        sink.emit(&event).expect("emit");

        let replay = store
            .replay_session(&SessionId::from("session-b"))
            .expect("replay")
            .expect("session exists");
        assert_eq!(replay.session.source, SessionSource::Cli);
        assert_eq!(replay.session.agent_id.as_deref(), Some("build"));
        assert_eq!(replay.agent_events, vec![event]);
    }

    #[test]
    fn event_sink_does_not_clear_existing_session_tree_metadata() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = Arc::new(SqliteSessionStore::new(temp.path()));
        let session_id = SessionId::from("session-tree");
        let mut parent = SessionRecord::new("parent-session", SessionSource::Cli);
        parent.agent_id = Some("parent-agent".into());
        store.upsert_session(&parent).expect("parent session");
        let mut session = SessionRecord::new(session_id.clone(), SessionSource::Cli);
        session.parent_session_id = Some(parent.session_id.clone());
        store.upsert_session(&session).expect("session");
        let leaf = SessionEntry::new(session_id.clone(), SessionEntryKind::Leaf);
        let leaf_id = leaf.entry_id.clone();
        store.append_entry(&leaf).expect("leaf");

        let sink = PersistingAgentEventSink::new(store.clone());
        let event = AgentEvent::new(
            session_id.clone(),
            "turn-tree",
            None,
            AgentEventSource::Runtime,
            AgentEventKind::TurnEnd,
            serde_json::Value::Null,
        );
        sink.emit(&event).expect("emit");

        let replay = store
            .replay_session(&session_id)
            .expect("replay")
            .expect("session exists");
        assert_eq!(replay.session.parent_session_id, Some(parent.session_id));
        assert_eq!(replay.session.active_leaf_entry_id, Some(leaf_id));
    }

    #[test]
    fn session_writer_preserves_event_order_for_one_turn() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("session-writer-order");
        let turn_id = TurnId::from("turn-writer-order");
        let session = sample_session(session_id.clone());
        let events = sample_events(session_id.clone(), turn_id.clone());

        let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
        for event in &events {
            writer.append_agent_event(event).expect("event");
        }
        writer.commit().expect("commit");

        let replay = store
            .replay_session(&session_id)
            .expect("replay")
            .expect("session exists");
        let stored_ids = replay
            .agent_events
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<Vec<_>>();
        let expected_ids = events
            .iter()
            .map(|event| event.event_id.clone())
            .collect::<Vec<_>>();
        assert_eq!(stored_ids, expected_ids);
    }

    #[test]
    fn session_writer_event_write_does_not_clear_active_leaf() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("session-writer-leaf");
        let turn_id = TurnId::from("turn-writer-leaf");
        let session = sample_session(session_id.clone());
        store.upsert_session(&session).expect("session");
        let leaf = sample_entry(
            session_id.clone(),
            turn_id.clone(),
            SessionEntryKind::Leaf,
            "leaf",
        );
        let leaf_id = leaf.entry_id.clone();
        store.append_entry(&leaf).expect("leaf");
        let event = sample_events(session_id.clone(), turn_id.clone())
            .into_iter()
            .next()
            .expect("event");

        let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
        writer.append_agent_event(&event).expect("event");
        writer.commit().expect("commit");

        let replay = store
            .replay_session(&session_id)
            .expect("replay")
            .expect("session exists");
        assert_eq!(replay.session.active_leaf_entry_id, Some(leaf_id));
    }

    #[test]
    fn session_writer_rolls_back_after_mid_turn_failure() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("session-writer-rollback");
        let turn_id = TurnId::from("turn-writer-rollback");
        let session = sample_session(session_id.clone());
        let event = sample_events(session_id.clone(), turn_id.clone())
            .into_iter()
            .next()
            .expect("event");
        let wrong_session_event = AgentEvent::new(
            "other-session",
            turn_id.clone(),
            None,
            AgentEventSource::Runtime,
            AgentEventKind::TurnEnd,
            serde_json::Value::Null,
        );

        let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
        writer.append_agent_event(&event).expect("event");
        assert!(writer.append_agent_event(&wrong_session_event).is_err());
        assert!(writer.commit().is_err());

        assert!(store.replay_session(&session_id).expect("replay").is_none());
    }

    #[test]
    fn session_writer_replay_matches_one_shot_writes() {
        let one_shot_temp = tempfile::tempdir().expect("one shot tempdir");
        let writer_temp = tempfile::tempdir().expect("writer tempdir");
        let one_shot = SqliteSessionStore::new(one_shot_temp.path());
        let writer_store = SqliteSessionStore::new(writer_temp.path());
        let session_id = SessionId::from("session-writer-parity");
        let turn_id = TurnId::from("turn-writer-parity");
        let session = sample_session(session_id.clone());
        let user = sample_entry(
            session_id.clone(),
            turn_id.clone(),
            SessionEntryKind::UserMessage,
            "hello",
        );
        let mut assistant = sample_entry(
            session_id.clone(),
            turn_id.clone(),
            SessionEntryKind::AssistantMessage,
            "world",
        );
        assistant.parent_entry_id = Some(user.entry_id.clone());
        let events = sample_events(session_id.clone(), turn_id.clone());
        let approval = sample_approval(session_id.clone(), turn_id.clone());

        one_shot.upsert_session(&session).expect("session");
        one_shot.append_entry(&user).expect("user");
        one_shot.append_entry(&assistant).expect("assistant");
        for event in &events {
            one_shot.append_agent_event(event).expect("event");
        }
        one_shot.append_approval(&approval).expect("approval");

        let mut writer = writer_store.begin_turn(&session, &turn_id).expect("writer");
        writer.append_entry(&user).expect("user");
        writer.append_entry(&assistant).expect("assistant");
        for event in &events {
            writer.append_agent_event(event).expect("event");
        }
        writer.append_approval(&approval).expect("approval");
        writer.commit().expect("commit");

        let one_shot_replay = one_shot
            .replay_session(&session_id)
            .expect("one shot replay")
            .expect("session exists");
        let writer_replay = writer_store
            .replay_session(&session_id)
            .expect("writer replay")
            .expect("session exists");
        assert_eq!(writer_replay, one_shot_replay);
    }

    #[test]
    fn persisting_turn_sink_commits_events_in_one_transaction() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::new(temp.path()));
        let sink = PersistingAgentTurnSink::new(store.clone())
            .with_source(SessionSource::Cli)
            .with_agent_id("build");
        let session_id = SessionId::from("session-turn-sink");
        let turn_id = TurnId::from("turn-turn-sink");
        let events = sample_events(session_id.clone(), turn_id);

        for event in &events {
            sink.emit(event).expect("emit");
        }
        sink.commit().expect("commit");

        let replay = store
            .replay_session(&session_id)
            .expect("replay")
            .expect("session exists");
        assert_eq!(replay.session.agent_id.as_deref(), Some("build"));
        assert_eq!(replay.agent_events, events);
    }

    #[test]
    fn session_tree_reads_and_switches_active_branch() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("session-branch");
        let turn_id = TurnId::from("turn-branch");
        store
            .upsert_session(&sample_session(session_id.clone()))
            .expect("session");
        let root = sample_entry(
            session_id.clone(),
            turn_id.clone(),
            SessionEntryKind::UserMessage,
            "root",
        );
        store.append_entry(&root).expect("root");
        let mut first_child = sample_entry(
            session_id.clone(),
            turn_id.clone(),
            SessionEntryKind::AssistantMessage,
            "first child",
        );
        first_child.parent_entry_id = Some(root.entry_id.clone());
        store.append_entry(&first_child).expect("first child");
        let mut second_child = sample_entry(
            session_id.clone(),
            turn_id,
            SessionEntryKind::AssistantMessage,
            "second child",
        );
        second_child.parent_entry_id = Some(root.entry_id.clone());
        store.append_entry(&second_child).expect("second child");

        let branch = store
            .active_branch(&session_id)
            .expect("active branch")
            .expect("session exists");
        assert_eq!(branch.entries.len(), 2);
        assert_eq!(branch.entries[0].entry_id, root.entry_id);
        assert_eq!(branch.entries[1].entry_id, second_child.entry_id);

        store
            .set_active_leaf(&session_id, &first_child.entry_id)
            .expect("switch leaf");
        let branch = store
            .active_branch(&session_id)
            .expect("active branch")
            .expect("session exists");
        assert_eq!(
            branch
                .entries
                .iter()
                .map(|entry| entry.visible_text.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("root"), Some("first child")]
        );
    }

    #[test]
    fn session_tree_rejects_cross_session_active_leaf() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("session-active");
        let other_session_id = SessionId::from("session-other-active");
        let turn_id = TurnId::from("turn-active");
        store
            .upsert_session(&sample_session(session_id.clone()))
            .expect("session");
        store
            .upsert_session(&sample_session(other_session_id.clone()))
            .expect("other session");
        let other = sample_entry(
            other_session_id,
            turn_id,
            SessionEntryKind::UserMessage,
            "other",
        );
        store.append_entry(&other).expect("other");

        let error = store
            .set_active_leaf(&session_id, &other.entry_id)
            .expect_err("cross-session active leaf");
        assert!(error.to_string().contains("belongs to session"));
    }

    #[test]
    fn session_tree_appends_branch_compaction_and_retry_entries() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("session-tree-ops");
        let turn_id = TurnId::from("turn-tree-ops");
        store
            .upsert_session(&sample_session(session_id.clone()))
            .expect("session");
        let root = sample_entry(
            session_id.clone(),
            turn_id,
            SessionEntryKind::UserMessage,
            "original user message",
        );
        store.append_entry(&root).expect("root");

        let branch = store
            .branch_from_entry(&SessionBranchSummaryInput {
                session_id: session_id.clone(),
                parent_entry_id: root.entry_id.clone(),
                summary: "try a shorter answer".into(),
                payload: json!({"reason": "user retry"}),
            })
            .expect("branch summary");
        assert_eq!(branch.kind, SessionEntryKind::BranchSummary);
        assert_eq!(branch.parent_entry_id, Some(root.entry_id.clone()));

        let compaction = store
            .append_compaction(&SessionCompactionInput {
                session_id: session_id.clone(),
                parent_entry_id: branch.entry_id.clone(),
                summary: "compressed prior context".into(),
                compacted_entry_ids: vec![root.entry_id.clone(), branch.entry_id.clone()],
                payload: json!({"tokens_saved": 128}),
            })
            .expect("compaction");
        assert_eq!(compaction.kind, SessionEntryKind::Compaction);
        assert_eq!(
            compaction.payload["compacted_entry_ids"],
            json!([root.entry_id.as_str(), branch.entry_id.as_str()])
        );

        let retry = store
            .retry_from_entry(&SessionRetryInput {
                session_id: session_id.clone(),
                parent_entry_id: compaction.entry_id.clone(),
                reason: Some("retry after compaction".into()),
                payload: json!({"attempt": 2}),
            })
            .expect("retry");
        assert_eq!(retry.kind, SessionEntryKind::Leaf);
        assert_eq!(retry.parent_entry_id, Some(compaction.entry_id.clone()));

        let replay = store
            .replay_session(&session_id)
            .expect("replay")
            .expect("session exists");
        assert_eq!(replay.session.active_leaf_entry_id, Some(retry.entry_id));
        let active_branch = store
            .active_branch(&session_id)
            .expect("active branch")
            .expect("session exists");
        assert_eq!(
            active_branch
                .entries
                .iter()
                .map(|entry| entry.kind)
                .collect::<Vec<_>>(),
            vec![
                SessionEntryKind::UserMessage,
                SessionEntryKind::BranchSummary,
                SessionEntryKind::Compaction,
                SessionEntryKind::Leaf,
            ]
        );
    }

    #[test]
    fn session_search_uses_fts_trigram_and_substring_indexes() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("session-search");
        let other_session_id = SessionId::from("session-other");
        let turn_id = TurnId::from("turn-search");
        store
            .upsert_session(&sample_session(session_id.clone()))
            .expect("session");
        store
            .upsert_session(&sample_session(other_session_id.clone()))
            .expect("other session");

        let english = sample_entry(
            session_id.clone(),
            turn_id.clone(),
            SessionEntryKind::UserMessage,
            "Prefer concise project search notes.",
        );
        let chinese = sample_entry(
            session_id.clone(),
            turn_id.clone(),
            SessionEntryKind::AssistantMessage,
            "中文搜索体验需要支持子串匹配。",
        );
        let other = sample_entry(
            other_session_id.clone(),
            turn_id.clone(),
            SessionEntryKind::UserMessage,
            "Prefer concise notes in another session.",
        );
        store.append_entry(&english).expect("english entry");
        store.append_entry(&chinese).expect("chinese entry");
        store.append_entry(&other).expect("other entry");

        let english_hits = store
            .search_entries(
                &SessionSearchQuery::new("concise")
                    .for_session(session_id.clone())
                    .with_limit(10),
            )
            .expect("english search");
        assert_eq!(english_hits.len(), 1);
        assert_eq!(english_hits[0].entry.entry_id, english.entry_id);
        assert_eq!(english_hits[0].index, SessionSearchIndex::Fts);
        assert!(english_hits[0].snippet.contains("concise"));

        let trigram_hits = store
            .search_entries(
                &SessionSearchQuery::new("搜索体验")
                    .for_session(session_id.clone())
                    .with_limit(10),
            )
            .expect("trigram search");
        assert_eq!(trigram_hits.len(), 1);
        assert_eq!(trigram_hits[0].entry.entry_id, chinese.entry_id);
        assert_eq!(trigram_hits[0].index, SessionSearchIndex::Trigram);
        assert!(trigram_hits[0].snippet.contains("搜索体验"));

        let short_cjk_hits = store
            .search_entries(
                &SessionSearchQuery::new("中文")
                    .for_session(session_id)
                    .with_limit(10),
            )
            .expect("short cjk search");
        assert_eq!(short_cjk_hits.len(), 1);
        assert_eq!(short_cjk_hits[0].entry.entry_id, chinese.entry_id);
        assert_eq!(short_cjk_hits[0].index, SessionSearchIndex::Substring);
    }

    #[test]
    fn session_search_indexes_turn_writer_entries_on_commit() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("session-writer-search");
        let turn_id = TurnId::from("turn-writer-search");
        let session = sample_session(session_id.clone());
        let entry = sample_entry(
            session_id.clone(),
            turn_id.clone(),
            SessionEntryKind::AssistantMessage,
            "writer committed searchable content",
        );

        let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
        writer.append_entry(&entry).expect("entry");
        writer.commit().expect("commit");

        let hits = store
            .search_entries(
                &SessionSearchQuery::new("searchable")
                    .for_session(session_id)
                    .with_limit(5),
            )
            .expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entry.entry_id, entry.entry_id);
    }

    #[test]
    fn session_search_does_not_index_rolled_back_writer_entries() {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = SqliteSessionStore::new(temp.path());
        let session_id = SessionId::from("session-writer-search-rollback");
        let turn_id = TurnId::from("turn-writer-search-rollback");
        let session = sample_session(session_id.clone());
        let entry = sample_entry(
            session_id.clone(),
            turn_id.clone(),
            SessionEntryKind::AssistantMessage,
            "rolled back searchable content",
        );

        let mut writer = store.begin_turn(&session, &turn_id).expect("writer");
        writer.append_entry(&entry).expect("entry");
        writer.rollback().expect("rollback");

        let hits = store
            .search_entries(
                &SessionSearchQuery::new("searchable")
                    .for_session(session_id)
                    .with_limit(5),
            )
            .expect("search");
        assert!(hits.is_empty());
    }
}
