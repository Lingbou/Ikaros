use crate::domain::{Message, Session, ToolCall, ToolInvocation};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use uuid::Uuid;

const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, Clone)]
pub struct SessionStore {
    path: PathBuf,
}

impl SessionStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let store = Self { path: path.into() };
        if let Some(parent) = store.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        store.connect()?;
        Ok(store)
    }

    pub fn create_session(&self, workspace: &Path, model: &str) -> Result<Session> {
        let workspace = workspace_string(workspace)?;
        let now = now_timestamp()?;
        let session = Session {
            id: Uuid::new_v4().to_string(),
            workspace,
            model: model.to_owned(),
            created_at: now,
            updated_at: now,
        };
        self.connect()?.execute(
            "INSERT INTO sessions (id, workspace, model, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                session.id,
                session.workspace,
                session.model,
                session.created_at,
                session.updated_at
            ],
        )?;
        Ok(session)
    }

    pub fn latest_session(&self, workspace: &Path) -> Result<Option<Session>> {
        let workspace = workspace_string(workspace)?;
        self.connect()?
            .query_row(
                "SELECT id, workspace, model, created_at, updated_at
                 FROM sessions WHERE workspace = ?1
                 ORDER BY updated_at DESC, created_at DESC, rowid DESC LIMIT 1",
                [workspace],
                session_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn start_turn(&self, session_id: &str, user_message: &Message) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let changed = transaction.execute(
            "UPDATE sessions SET active_turn = 1, updated_at = ?2
             WHERE id = ?1 AND active_turn = 0",
            params![session_id, now_timestamp()?],
        )?;
        if changed != 1 {
            bail!("session already has an active turn: {session_id}");
        }
        insert_message(&transaction, session_id, user_message)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn record_assistant_reply(&self, session_id: &str, message: &Message) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        insert_message(&transaction, session_id, message)?;
        let now = now_timestamp()?;
        for call in &message.tool_calls {
            let invocation_id = Uuid::new_v4().to_string();
            transaction.execute(
                "INSERT INTO tool_invocations
                 (id, provider_call_id, session_id, name, args_json, decision, status, output, error,
                  created_at, finished_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, 'pending', NULL, NULL, ?6, NULL)",
                params![
                    invocation_id,
                    call.id,
                    session_id,
                    call.name,
                    serde_json::to_string(&call.arguments)?,
                    now
                ],
            )?;
        }
        if message.tool_calls.is_empty() {
            finish_session_turn(&transaction, session_id)?;
        } else {
            touch_session(&transaction, session_id)?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let connection = self.connect()?;
        let mut statement = connection
            .prepare("SELECT payload_json FROM messages WHERE session_id = ?1 ORDER BY seq ASC")?;
        let rows = statement.query_map([session_id], |row| row.get::<_, String>(0))?;
        let mut messages = Vec::new();
        for row in rows {
            let payload = row?;
            messages.push(
                serde_json::from_str(&payload)
                    .with_context(|| format!("invalid stored message in session {session_id}"))?,
            );
        }
        Ok(messages)
    }

    pub fn pending_tool(&self, session_id: &str) -> Result<Option<ToolInvocation>> {
        self.tool_with_status(session_id, "pending")
    }

    pub fn interrupted_tool(&self, session_id: &str) -> Result<Option<ToolInvocation>> {
        self.tool_with_status(session_id, "executing")
    }

    pub fn approve_tool(&self, session_id: &str, invocation_id: &str) -> Result<ToolInvocation> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let invocation = load_tool(&transaction, session_id, invocation_id, "pending")?;
        let changed = transaction.execute(
            "UPDATE tool_invocations
             SET decision = 'approved', status = 'executing'
             WHERE session_id = ?1 AND id = ?2 AND status = 'pending'",
            params![session_id, invocation_id],
        )?;
        if changed != 1 {
            bail!("tool approval state changed concurrently: {invocation_id}");
        }
        transaction.commit()?;
        Ok(invocation)
    }

    pub fn deny_tool(&self, session_id: &str, invocation_id: &str) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let invocation = load_tool(&transaction, session_id, invocation_id, "pending")?;
        let content = "Tool call denied by the user.";
        let changed = transaction.execute(
            "UPDATE tool_invocations
             SET decision = 'denied', status = 'denied', output = ?3, finished_at = ?4
             WHERE session_id = ?1 AND id = ?2 AND status = 'pending'",
            params![session_id, invocation_id, content, now_timestamp()?],
        )?;
        if changed != 1 {
            bail!("tool denial state changed concurrently: {invocation_id}");
        }
        insert_message(
            &transaction,
            session_id,
            &Message::tool(&invocation.call.id, content),
        )?;
        touch_session(&transaction, session_id)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn finish_tool(
        &self,
        session_id: &str,
        invocation_id: &str,
        result: Result<String>,
    ) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let invocation = load_tool(&transaction, session_id, invocation_id, "executing")?;
        let (status, output, error, tool_message) = match result {
            Ok(output) => (
                "completed",
                Some(output.clone()),
                None,
                format!("Tool completed successfully.\n{output}"),
            ),
            Err(error) => {
                let error = format!("{error:#}");
                (
                    "failed",
                    None,
                    Some(error.clone()),
                    format!("Tool failed.\n{error}"),
                )
            }
        };
        let changed = transaction.execute(
            "UPDATE tool_invocations
             SET status = ?3, output = ?4, error = ?5, finished_at = ?6
             WHERE session_id = ?1 AND id = ?2 AND status = 'executing'",
            params![
                session_id,
                invocation_id,
                status,
                output,
                error,
                now_timestamp()?
            ],
        )?;
        if changed != 1 {
            bail!("tool completion state changed concurrently: {invocation_id}");
        }
        insert_message(
            &transaction,
            session_id,
            &Message::tool(&invocation.call.id, tool_message),
        )?;
        touch_session(&transaction, session_id)?;
        transaction.commit()?;
        Ok(())
    }

    pub fn mark_interrupted_failed(&self, session_id: &str, invocation_id: &str) -> Result<()> {
        let mut connection = self.connect()?;
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let invocation = load_tool(&transaction, session_id, invocation_id, "executing")?;
        let error = "Tool execution was interrupted. Its side effects are unknown, so Ikaros did not replay it.";
        transaction.execute(
            "UPDATE tool_invocations
             SET status = 'failed', error = ?3, finished_at = ?4
             WHERE session_id = ?1 AND id = ?2 AND status = 'executing'",
            params![session_id, invocation_id, error, now_timestamp()?],
        )?;
        insert_message(
            &transaction,
            session_id,
            &Message::tool(&invocation.call.id, error),
        )?;
        touch_session(&transaction, session_id)?;
        transaction.commit()?;
        Ok(())
    }

    fn tool_with_status(&self, session_id: &str, status: &str) -> Result<Option<ToolInvocation>> {
        let connection = self.connect()?;
        connection
            .query_row(
                "SELECT id, provider_call_id, name, args_json
                 FROM tool_invocations
                 WHERE session_id = ?1 AND status = ?2
                 ORDER BY created_at ASC, rowid ASC LIMIT 1",
                params![session_id, status],
                tool_from_row,
            )
            .optional()
            .map_err(Into::into)
    }

    fn connect(&self) -> Result<Connection> {
        let mut connection = Connection::open(&self.path)
            .with_context(|| format!("failed to open {}", self.path.display()))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", true)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "NORMAL")?;
        let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
        match version {
            0 => {
                let transaction =
                    connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let locked_version: i64 =
                    transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
                match locked_version {
                    0 => {
                        transaction.execute_batch(
                            "CREATE TABLE sessions (
                         id TEXT PRIMARY KEY,
                         workspace TEXT NOT NULL,
                         model TEXT NOT NULL,
                         created_at INTEGER NOT NULL,
                         updated_at INTEGER NOT NULL,
                         active_turn INTEGER NOT NULL DEFAULT 0 CHECK(active_turn IN (0, 1))
                     );
                     CREATE INDEX sessions_workspace_updated
                         ON sessions(workspace, updated_at DESC);
                     CREATE TABLE messages (
                         id INTEGER PRIMARY KEY AUTOINCREMENT,
                         session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                         seq INTEGER NOT NULL,
                         payload_json TEXT NOT NULL,
                         created_at INTEGER NOT NULL,
                         UNIQUE(session_id, seq)
                     );
                     CREATE TABLE tool_invocations (
                         id TEXT PRIMARY KEY,
                         provider_call_id TEXT NOT NULL,
                         session_id TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
                         name TEXT NOT NULL,
                         args_json TEXT NOT NULL,
                         decision TEXT,
                         status TEXT NOT NULL CHECK(status IN
                             ('pending', 'executing', 'completed', 'failed', 'denied')),
                         output TEXT,
                         error TEXT,
                         created_at INTEGER NOT NULL,
                         finished_at INTEGER
                     );
                     CREATE INDEX tool_invocations_session_status
                         ON tool_invocations(session_id, status, created_at);",
                        )?;
                        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
                    }
                    SCHEMA_VERSION => {}
                    other => {
                        bail!(
                            "unsupported session database schema {other}; expected {SCHEMA_VERSION}"
                        )
                    }
                }
                transaction.commit()?;
            }
            SCHEMA_VERSION => {}
            other => {
                bail!("unsupported session database schema {other}; expected {SCHEMA_VERSION}")
            }
        }
        Ok(connection)
    }
}

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        workspace: row.get(1)?,
        model: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
    })
}

fn tool_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ToolInvocation> {
    let args_json: String = row.get(3)?;
    let arguments = serde_json::from_str(&args_json).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(ToolInvocation {
        id: row.get(0)?,
        call: ToolCall {
            id: row.get(1)?,
            name: row.get(2)?,
            arguments,
        },
    })
}

fn load_tool(
    transaction: &Transaction<'_>,
    session_id: &str,
    invocation_id: &str,
    status: &str,
) -> Result<ToolInvocation> {
    transaction
        .query_row(
            "SELECT id, provider_call_id, name, args_json FROM tool_invocations
             WHERE session_id = ?1 AND id = ?2 AND status = ?3",
            params![session_id, invocation_id, status],
            tool_from_row,
        )
        .optional()?
        .ok_or_else(|| anyhow::anyhow!("tool invocation {invocation_id} is not {status}"))
}

fn insert_message(
    transaction: &Transaction<'_>,
    session_id: &str,
    message: &Message,
) -> Result<()> {
    let next_seq: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(seq), 0) + 1 FROM messages WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )?;
    transaction.execute(
        "INSERT INTO messages (session_id, seq, payload_json, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            session_id,
            next_seq,
            serde_json::to_string(message)?,
            now_timestamp()?
        ],
    )?;
    Ok(())
}

fn touch_session(transaction: &Transaction<'_>, session_id: &str) -> Result<()> {
    let changed = transaction.execute(
        "UPDATE sessions SET updated_at = ?2 WHERE id = ?1",
        params![session_id, now_timestamp()?],
    )?;
    if changed != 1 {
        bail!("unknown session: {session_id}");
    }
    Ok(())
}

fn finish_session_turn(transaction: &Transaction<'_>, session_id: &str) -> Result<()> {
    let changed = transaction.execute(
        "UPDATE sessions SET active_turn = 0, updated_at = ?2 WHERE id = ?1",
        params![session_id, now_timestamp()?],
    )?;
    if changed != 1 {
        bail!("unknown session: {session_id}");
    }
    Ok(())
}

fn workspace_string(workspace: &Path) -> Result<String> {
    let canonical = workspace
        .canonicalize()
        .with_context(|| format!("failed to resolve workspace {}", workspace.display()))?;
    Ok(canonical.to_string_lossy().into_owned())
}

fn now_timestamp() -> Result<i64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?;
    i64::try_from(duration.as_millis()).context("system timestamp does not fit in SQLite integer")
}

#[cfg(test)]
mod tests {
    use super::SessionStore;
    use crate::domain::{Message, ToolCall};
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn messages_survive_reopen() {
        let temp = tempdir().expect("tempdir");
        let database = temp.path().join("sessions.sqlite3");
        let store = SessionStore::open(&database).expect("store");
        let session = store
            .create_session(temp.path(), "test-model")
            .expect("session");
        store
            .start_turn(&session.id, &Message::user("hello"))
            .expect("start turn");
        drop(store);

        let reopened = SessionStore::open(&database).expect("reopen");
        assert_eq!(
            reopened.messages(&session.id).expect("messages"),
            vec![Message::user("hello")]
        );
    }

    #[test]
    fn latest_session_has_a_stable_tiebreaker() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("store");
        let first = store
            .create_session(temp.path(), "first-model")
            .expect("first session");
        let second = store
            .create_session(temp.path(), "second-model")
            .expect("second session");
        store
            .connect()
            .expect("connection")
            .execute("UPDATE sessions SET created_at = 1, updated_at = 1", [])
            .expect("tie timestamps");
        let latest = store
            .latest_session(temp.path())
            .expect("latest query")
            .expect("latest session");
        assert_ne!(first.id, second.id);
        assert_eq!(latest.id, second.id);
    }

    #[test]
    fn pending_tool_is_durable() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("store");
        let session = store
            .create_session(temp.path(), "test-model")
            .expect("session");
        let message = Message::assistant(
            None,
            vec![ToolCall {
                id: "call-1".to_owned(),
                name: "read_file".to_owned(),
                arguments: json!({"path": "README.md"}),
            }],
        );
        store
            .record_assistant_reply(&session.id, &message)
            .expect("record");
        drop(store);

        let reopened = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("reopen");
        let pending = reopened
            .pending_tool(&session.id)
            .expect("pending")
            .expect("tool");
        assert_eq!(pending.call.id, "call-1");
        assert_eq!(pending.call.arguments, json!({"path": "README.md"}));
    }

    #[test]
    fn provider_call_ids_may_repeat_across_turns() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("store");
        let session = store
            .create_session(temp.path(), "test-model")
            .expect("session");
        for path in ["one.txt", "two.txt"] {
            store
                .record_assistant_reply(
                    &session.id,
                    &Message::assistant(
                        None,
                        vec![ToolCall {
                            id: "call_0".to_owned(),
                            name: "read_file".to_owned(),
                            arguments: json!({"path": path}),
                        }],
                    ),
                )
                .expect("reply");
            let pending = store
                .pending_tool(&session.id)
                .expect("pending query")
                .expect("pending tool");
            store.deny_tool(&session.id, &pending.id).expect("deny");
        }
    }

    #[test]
    fn only_one_active_turn_can_start_per_session() {
        let temp = tempdir().expect("tempdir");
        let store = SessionStore::open(temp.path().join("sessions.sqlite3")).expect("store");
        let session = store
            .create_session(temp.path(), "test-model")
            .expect("session");
        store
            .start_turn(&session.id, &Message::user("first"))
            .expect("first turn");
        assert!(
            store
                .start_turn(&session.id, &Message::user("second"))
                .is_err()
        );
        store
            .record_assistant_reply(
                &session.id,
                &Message::assistant(Some("done".to_owned()), Vec::new()),
            )
            .expect("finish");
        store
            .start_turn(&session.id, &Message::user("next"))
            .expect("next turn");
    }
}
