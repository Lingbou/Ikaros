// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AgentEvent, AgentEventKind, ApprovalRecord, ApprovalStatus, SessionEntry, SessionEntryId,
    SessionEntryKind, SessionId, SessionRecord, SessionSource, SessionStore, TurnId,
};
use ikaros_core::{IkarosError, Result};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const SESSION_SCHEMA_VERSION: i64 = 1;

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

            PRAGMA user_version = 1;
            "#,
        )
        .map_err(|source| sqlite_error(&self.path, source))
    }

    fn insert_missing_session(&self, conn: &Connection, session_id: &SessionId) -> Result<()> {
        let started_at = format_time(OffsetDateTime::now_utc())?;
        let source_json = serde_json::to_string(&SessionSource::Runtime)?;
        conn.execute(
            "INSERT OR IGNORE INTO sessions (id, source_json, started_at) VALUES (?1, ?2, ?3)",
            params![session_id.as_str(), source_json, started_at],
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(())
    }
}

impl SessionStore for SqliteSessionStore {
    fn upsert_session(&self, session: &SessionRecord) -> Result<()> {
        let conn = self.open()?;
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
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(())
    }

    fn finish_session(&self, session_id: &SessionId, ended_at: OffsetDateTime) -> Result<()> {
        let conn = self.open()?;
        self.insert_missing_session(&conn, session_id)?;
        conn.execute(
            "UPDATE sessions SET ended_at = ?1 WHERE id = ?2",
            params![format_time(ended_at)?, session_id.as_str()],
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(())
    }

    fn append_entry(&self, entry: &SessionEntry) -> Result<()> {
        let conn = self.open()?;
        self.insert_missing_session(&conn, &entry.session_id)?;
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
        .map_err(|source| sqlite_error(&self.path, source))?;
        conn.execute(
            "UPDATE sessions SET active_leaf_entry_id = ?1 WHERE id = ?2",
            params![entry.entry_id.as_str(), entry.session_id.as_str()],
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(())
    }

    fn append_agent_event(&self, event: &AgentEvent) -> Result<()> {
        let conn = self.open()?;
        self.insert_missing_session(&conn, &event.session_id)?;
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
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(())
    }

    fn append_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        let conn = self.open()?;
        self.insert_missing_session(&conn, &approval.session_id)?;
        conn.execute(
            r#"
            INSERT INTO approvals (
                id, session_id, turn_id, at, status, request_json, decision_json
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
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
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(())
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
            let (id, session_id, parent_entry_id, turn_id, at, kind, visible_text, payload_json) =
                row.map_err(|source| sqlite_error(&self.path, source))?;
            entries.push(SessionEntry {
                entry_id: SessionEntryId::from(id),
                session_id: SessionId::from(session_id),
                parent_entry_id: parent_entry_id.map(SessionEntryId::from),
                turn_id: turn_id.map(TurnId::from),
                at: parse_time(&at)?,
                kind: entry_kind_from_str(&kind)?,
                visible_text,
                payload: serde_json::from_str(&payload_json)?,
            });
        }
        Ok(entries)
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
            let (approval_id, session_id, turn_id, at, status, request_json, decision_json) =
                row.map_err(|source| sqlite_error(&self.path, source))?;
            approvals.push(ApprovalRecord {
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
            });
        }
        Ok(approvals)
    }
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
        PersistingAgentEventSink,
    };
    use ikaros_models::ModelStreamEvent;
    use serde_json::json;
    use std::sync::Arc;

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
}
