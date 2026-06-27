// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(super) struct SqliteSessionWriter {
    pub(super) conn: Option<Connection>,
    pub(super) path: PathBuf,
    pub(super) session_id: SessionId,
    pub(super) turn_id: TurnId,
    pub(super) failed: bool,
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
    fn promote_input(&mut self, input_id: &SessionInputId) -> Result<()> {
        let result =
            promote_input(self.conn()?, &self.path, input_id, &self.turn_id).and_then(|input| {
                input
                    .map(|_| ())
                    .ok_or_else(|| IkarosError::Message("session input is not admitted".into()))
            });
        self.mark(result)
    }

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
        let mut turn = SessionTurnRecord::new(self.session_id.clone(), self.turn_id.clone());
        turn.status = SessionTurnStatus::Completed;
        turn.completed_at = Some(OffsetDateTime::now_utc());
        if let Err(error) = upsert_turn(&conn, &self.path, &turn) {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(error);
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
