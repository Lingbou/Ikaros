// SPDX-License-Identifier: GPL-3.0-only

use super::*;

mod maintenance;
mod schema;

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

    pub fn session_records(&self) -> Result<Vec<SessionRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        session_records(&conn, &self.path)
    }

    fn open(&self) -> Result<Connection> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
        }
        let conn =
            Connection::open(&self.path).map_err(|source| sqlite_error(&self.path, source))?;
        conn.busy_timeout(StdDuration::from_millis(SQLITE_BUSY_TIMEOUT_MS))
            .map_err(|source| sqlite_error(&self.path, source))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|source| sqlite_error(&self.path, source))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|source| sqlite_error(&self.path, source))?;
        schema::ensure_schema(&conn, &self.path)?;
        Ok(conn)
    }

    fn pre_restore_backup_path(&self) -> PathBuf {
        let timestamp = OffsetDateTime::now_utc().unix_timestamp_nanos();
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state.db");
        self.path
            .with_file_name(format!("{file_name}.pre-restore-{timestamp}.bak"))
    }

    fn restore_temp_path(&self) -> PathBuf {
        let timestamp = OffsetDateTime::now_utc().unix_timestamp_nanos();
        let file_name = self
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state.db");
        self.path
            .with_file_name(format!(".{file_name}.restore-{timestamp}.tmp"))
    }

    fn remove_sidecar_files(&self) -> Result<()> {
        for sidecar in [
            sqlite_sidecar_path(&self.path, "wal"),
            sqlite_sidecar_path(&self.path, "shm"),
        ] {
            match fs::remove_file(&sidecar) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(source) => return Err(IkarosError::io(&sidecar, source)),
            }
        }
        Ok(())
    }

    fn with_write_transaction<T>(
        &self,
        operation: &'static str,
        f: impl FnOnce(&Connection) -> Result<T>,
    ) -> Result<T> {
        let conn = self.open()?;
        begin_immediate_transaction(&conn, &self.path, operation)?;
        let result = f(&conn);
        match result {
            Ok(value) => {
                commit_transaction(&conn, &self.path, operation)?;
                Ok(value)
            }
            Err(error) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
    }
}

impl SessionStore for SqliteSessionStore {
    fn upsert_session(&self, session: &SessionRecord) -> Result<()> {
        self.with_write_transaction("upsert_session", |conn| {
            upsert_session(conn, &self.path, session)
        })
    }

    fn finish_session(&self, session_id: &SessionId, ended_at: OffsetDateTime) -> Result<()> {
        self.with_write_transaction("finish_session", |conn| {
            finish_session(conn, &self.path, session_id, ended_at)
        })
    }

    fn begin_turn(
        &self,
        session: &SessionRecord,
        turn_id: &TurnId,
    ) -> Result<Box<dyn SessionWriter>> {
        let conn = self.open()?;
        begin_immediate_transaction(&conn, &self.path, "begin_turn")?;
        upsert_session(&conn, &self.path, session)?;
        let mut turn = SessionTurnRecord::new(session.session_id.clone(), turn_id.clone());
        turn.status = SessionTurnStatus::Running;
        upsert_turn(&conn, &self.path, &turn)?;
        Ok(Box::new(SqliteSessionWriter {
            conn: Some(conn),
            path: self.path.clone(),
            session_id: session.session_id.clone(),
            turn_id: turn_id.clone(),
            failed: false,
        }))
    }

    fn append_entry(&self, entry: &SessionEntry) -> Result<()> {
        self.with_write_transaction("append_entry", |conn| append_entry(conn, &self.path, entry))
    }

    fn append_agent_event(&self, event: &AgentEvent) -> Result<()> {
        self.with_write_transaction("append_agent_event", |conn| {
            append_agent_event(conn, &self.path, event)
        })
    }

    fn append_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        self.with_write_transaction("append_approval", |conn| {
            append_approval(conn, &self.path, approval)
        })
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
        self.with_write_transaction("set_active_leaf", |conn| {
            set_active_leaf(conn, &self.path, session_id, entry_id)
        })
    }

    fn append_branch_summary(&self, input: &SessionBranchSummaryInput) -> Result<SessionEntry> {
        self.with_write_transaction("append_branch_summary", |conn| {
            append_branch_summary(conn, &self.path, input)
        })
    }

    fn append_compaction(&self, input: &SessionCompactionInput) -> Result<SessionEntry> {
        self.with_write_transaction("append_compaction", |conn| {
            append_compaction(conn, &self.path, input)
        })
    }

    fn append_retry_marker(&self, input: &SessionRetryInput) -> Result<SessionEntry> {
        self.with_write_transaction("append_retry_marker", |conn| {
            append_retry_marker(conn, &self.path, input)
        })
    }

    fn admit_input(&self, input: &SessionInputAdmission) -> Result<SessionInput> {
        self.with_write_transaction("admit_input", |conn| admit_input(conn, &self.path, input))
    }

    fn promote_input(
        &self,
        input_id: &SessionInputId,
        turn_id: &TurnId,
    ) -> Result<Option<SessionInput>> {
        self.with_write_transaction("promote_input", |conn| {
            promote_input(conn, &self.path, input_id, turn_id)
        })
    }

    fn cancel_input(
        &self,
        input_id: &SessionInputId,
        reason: &str,
    ) -> Result<Option<SessionInput>> {
        self.with_write_transaction("cancel_input", |conn| {
            cancel_input(conn, &self.path, input_id, reason)
        })
    }

    fn session_inputs(&self, session_id: &SessionId) -> Result<Vec<SessionInput>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        session_inputs(&conn, &self.path, session_id)
    }

    fn upsert_turn(&self, turn: &SessionTurnRecord) -> Result<()> {
        self.with_write_transaction("upsert_turn", |conn| upsert_turn(conn, &self.path, turn))
    }

    fn session_turn(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) -> Result<Option<SessionTurnRecord>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let conn = self.open()?;
        session_turn(&conn, &self.path, session_id, turn_id)
    }

    fn session_turns(&self, session_id: &SessionId) -> Result<Vec<SessionTurnRecord>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        session_turns(&conn, &self.path, session_id)
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

    fn enqueue_continuation(
        &self,
        input: &SessionContinuationInput,
    ) -> Result<SessionContinuation> {
        self.with_write_transaction("enqueue_continuation", |conn| {
            enqueue_continuation(conn, &self.path, input)
        })
    }

    fn claim_next_continuation(
        &self,
        claim: &SessionContinuationClaim,
    ) -> Result<Option<SessionContinuation>> {
        self.with_write_transaction("claim_next_continuation", |conn| {
            claim_next_continuation_in_transaction(conn, &self.path, claim)
        })
    }

    fn complete_continuation(
        &self,
        continuation_id: &ContinuationId,
        payload: serde_json::Value,
    ) -> Result<Option<SessionContinuation>> {
        self.with_write_transaction("complete_continuation", |conn| {
            update_continuation_status(
                conn,
                &self.path,
                continuation_id,
                SessionContinuationStatus::Completed,
                Some(payload),
                None,
            )
        })
    }

    fn fail_continuation(
        &self,
        continuation_id: &ContinuationId,
        error: &str,
    ) -> Result<Option<SessionContinuation>> {
        self.with_write_transaction("fail_continuation", |conn| {
            update_continuation_status(
                conn,
                &self.path,
                continuation_id,
                SessionContinuationStatus::Failed,
                None,
                Some(error),
            )
        })
    }

    fn cancel_continuation(
        &self,
        continuation_id: &ContinuationId,
        reason: &str,
    ) -> Result<Option<SessionContinuation>> {
        self.with_write_transaction("cancel_continuation", |conn| {
            update_continuation_status(
                conn,
                &self.path,
                continuation_id,
                SessionContinuationStatus::Cancelled,
                None,
                Some(reason),
            )
        })
    }

    fn requeue_continuation(
        &self,
        continuation_id: &ContinuationId,
        reason: &str,
        payload: serde_json::Value,
    ) -> Result<Option<SessionContinuation>> {
        self.with_write_transaction("requeue_continuation", |conn| {
            requeue_continuation(conn, &self.path, continuation_id, reason, payload)
        })
    }

    fn continuations(&self, session_id: &SessionId) -> Result<Vec<SessionContinuation>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        continuations_for_session(&conn, &self.path, session_id)
    }

    fn session_timeline(&self, session_id: &SessionId) -> Result<Vec<SessionTimelineItem>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let conn = self.open()?;
        session_timeline(&conn, &self.path, session_id)
    }

    fn session_timeline_page(&self, query: &SessionTimelineQuery) -> Result<SessionTimelinePage> {
        if !self.path.exists() {
            let page = query.page.max(1);
            let page_size = query.page_size.max(1);
            return Ok(SessionTimelinePage {
                session_id: query.session_id.clone(),
                turn_id: query.turn_id.clone(),
                page,
                page_size,
                total_items: 0,
                items: Vec::new(),
            });
        }
        let conn = self.open()?;
        session_timeline_page(&conn, &self.path, query)
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
                ORDER BY event_seq ASC, rowid ASC
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
            events.push(agent_event_from_parts(
                row.map_err(|source| sqlite_error(&self.path, source))?,
            )?);
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

    fn replay_session_page(
        &self,
        session_id: &SessionId,
        page: usize,
        page_size: usize,
    ) -> Result<Option<SessionReplayPage>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let conn = self.open()?;
        let Some(session) = session_record(&conn, &self.path, session_id)? else {
            return Ok(None);
        };
        let page = page.max(1);
        let page_size = page_size.max(1);
        let offset = page.saturating_sub(1).saturating_mul(page_size);
        Ok(Some(SessionReplayPage {
            session,
            page,
            page_size,
            total_entries: count_session_rows(&conn, &self.path, "session_entries", session_id)?,
            total_agent_events: count_session_rows(&conn, &self.path, "agent_events", session_id)?,
            total_approvals: count_session_rows(&conn, &self.path, "approvals", session_id)?,
            entries: session_entries_page(&conn, &self.path, session_id, offset, page_size)?,
            agent_events: agent_events_page(&conn, &self.path, session_id, offset, page_size)?,
            approvals: approvals_page(&conn, &self.path, session_id, offset, page_size)?,
        }))
    }
}
