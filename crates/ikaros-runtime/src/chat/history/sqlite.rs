// SPDX-License-Identifier: GPL-3.0-only

use super::{ChatHistoryRecord, ChatHistoryStore};
use ikaros_core::{IkarosError, Result};
use rusqlite::{Connection, params};
use std::{fs, path::Path};

impl ChatHistoryStore {
    pub(super) fn append_sqlite(&self, record: &ChatHistoryRecord) -> Result<()> {
        let conn = self.open_sqlite()?;
        conn.execute(
            "INSERT INTO chat_history (
                session_id, turn_id, created_at, agent, provider, model, streamed,
                user_message, assistant_message, relationship_hits, memory_hits, rag_hits
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                record.session_id,
                record.turn_id,
                record.created_at,
                record.agent,
                record.provider,
                record.model,
                i64::from(record.streamed),
                record.user_message,
                record.assistant_message,
                record.relationship_hits as i64,
                record.memory_hits as i64,
                record.rag_hits as i64,
            ],
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(())
    }

    pub(super) fn read_all_sqlite(&self) -> Result<Vec<ChatHistoryRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open_sqlite()?;
        let mut stmt = conn
            .prepare(
                "SELECT session_id, turn_id, created_at, agent, provider, model, streamed,
                        user_message, assistant_message, relationship_hits, memory_hits, rag_hits
                 FROM chat_history
                 ORDER BY sequence ASC",
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ChatHistoryRecord {
                    session_id: row.get(0)?,
                    turn_id: row.get(1)?,
                    created_at: row.get(2)?,
                    agent: row.get(3)?,
                    provider: row.get(4)?,
                    model: row.get(5)?,
                    streamed: row.get::<_, i64>(6)? != 0,
                    user_message: row.get(7)?,
                    assistant_message: row.get(8)?,
                    relationship_hits: row.get::<_, i64>(9)? as usize,
                    memory_hits: row.get::<_, i64>(10)? as usize,
                    rag_hits: row.get::<_, i64>(11)? as usize,
                })
            })
            .map_err(|source| sqlite_error(&self.path, source))?;
        let mut records = Vec::new();
        for row in rows {
            records.push(row.map_err(|source| sqlite_error(&self.path, source))?);
        }
        Ok(records)
    }

    pub(super) fn delete_session_sqlite(&self, session_id: &str) -> Result<usize> {
        if !self.path.exists() {
            return Ok(0);
        }
        let conn = self.open_sqlite()?;
        let deleted = conn
            .execute(
                "DELETE FROM chat_history WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(deleted)
    }

    pub(super) fn clear_sqlite(&self) -> Result<usize> {
        if !self.path.exists() {
            return Ok(0);
        }
        let conn = self.open_sqlite()?;
        let deleted = conn
            .execute("DELETE FROM chat_history", [])
            .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(deleted)
    }

    fn open_sqlite(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let conn =
            Connection::open(&self.path).map_err(|source| sqlite_error(&self.path, source))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|source| sqlite_error(&self.path, source))?;
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS chat_history (
                sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                turn_id TEXT NOT NULL UNIQUE,
                created_at TEXT NOT NULL,
                agent TEXT NOT NULL,
                provider TEXT NOT NULL,
                model TEXT NOT NULL,
                streamed INTEGER NOT NULL DEFAULT 0,
                user_message TEXT NOT NULL,
                assistant_message TEXT NOT NULL,
                relationship_hits INTEGER NOT NULL DEFAULT 0,
                memory_hits INTEGER NOT NULL DEFAULT 0,
                rag_hits INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS chat_history_session_idx ON chat_history(session_id, sequence);
            CREATE INDEX IF NOT EXISTS chat_history_created_at_idx ON chat_history(created_at);
            "#,
        )
        .map_err(|source| sqlite_error(&self.path, source))?;
        Ok(conn)
    }
}

fn sqlite_error(path: &Path, source: rusqlite::Error) -> IkarosError {
    IkarosError::Message(format!("sqlite error at {}: {source}", path.display()))
}
