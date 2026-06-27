// SPDX-License-Identifier: GPL-3.0-only

use super::super::*;

pub(super) fn ensure_schema(conn: &Connection, path: &Path) -> Result<()> {
    let version = conn
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|source| sqlite_error(path, source))?;
    if version == 0 && database_has_user_schema(conn, path)? {
        return Err(IkarosError::Message(format!(
            "state.db has no schema version but already contains tables; delete {} to let Ikaros create a fresh pre-release database",
            path.display()
        )));
    }
    if version != 0 && version != SESSION_SCHEMA_VERSION {
        return Err(IkarosError::Message(format!(
            "state.db schema version {version} is not supported; current version is {SESSION_SCHEMA_VERSION}. Delete {} to let Ikaros create a fresh pre-release database",
            path.display()
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

            CREATE TABLE IF NOT EXISTS session_turns (
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                completed_at TEXT,
                lease_owner TEXT,
                lease_expires_at TEXT,
                error TEXT,
                PRIMARY KEY(session_id, turn_id),
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            );
            CREATE INDEX IF NOT EXISTS session_turns_session_status_idx
                ON session_turns(session_id, status, started_at);

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
                event_seq INTEGER NOT NULL DEFAULT 0,
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
            CREATE INDEX IF NOT EXISTS agent_events_session_seq_idx
                ON agent_events(session_id, event_seq);

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

            CREATE TABLE IF NOT EXISTS session_timeline_items (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                turn_id TEXT,
                at TEXT NOT NULL,
                item_kind TEXT NOT NULL,
                item_id TEXT NOT NULL,
                UNIQUE(item_kind, item_id),
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            );
            CREATE INDEX IF NOT EXISTS session_timeline_session_seq_idx
                ON session_timeline_items(session_id, sequence);
            CREATE INDEX IF NOT EXISTS session_timeline_turn_idx
                ON session_timeline_items(turn_id);

            CREATE TABLE IF NOT EXISTS session_inputs (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                status TEXT NOT NULL,
                idempotency_key_digest TEXT,
                payload_json TEXT NOT NULL,
                admitted_at TEXT NOT NULL,
                promoted_turn_id TEXT,
                promoted_at TEXT,
                cancelled_at TEXT,
                cancel_reason TEXT,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            );
            CREATE INDEX IF NOT EXISTS session_inputs_session_status_idx
                ON session_inputs(session_id, status, admitted_at);
            CREATE INDEX IF NOT EXISTS session_inputs_idempotency_idx
                ON session_inputs(session_id, idempotency_key_digest);

            CREATE TABLE IF NOT EXISTS session_continuations (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                turn_id TEXT,
                parent_continuation_id TEXT,
                kind TEXT NOT NULL,
                status TEXT NOT NULL,
                status_reason TEXT,
                priority INTEGER NOT NULL,
                payload_json TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                claimed_at TEXT,
                completed_at TEXT,
                lease_owner TEXT,
                lease_expires_at TEXT,
                attempt_count INTEGER NOT NULL DEFAULT 0,
                error TEXT,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            );
            CREATE INDEX IF NOT EXISTS session_continuations_session_status_idx
                ON session_continuations(session_id, status, priority, created_at);
            CREATE INDEX IF NOT EXISTS session_continuations_turn_idx
                ON session_continuations(turn_id);
            CREATE INDEX IF NOT EXISTS session_continuations_parent_idx
                ON session_continuations(parent_continuation_id);
            "#,
    )
    .map_err(|source| sqlite_error(path, source))?;
    rebuild_missing_entry_search_indexes(conn, path)?;
    conn.pragma_update(None, "user_version", SESSION_SCHEMA_VERSION)
        .map_err(|source| sqlite_error(path, source))
}
