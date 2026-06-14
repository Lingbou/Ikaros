// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AgentEvent, AgentEventSink, ApprovalRecord, SessionEntry, SessionId, SessionRecord,
    SessionSource, SessionStore, SessionWriter, TurnId,
};
use ikaros_core::{IkarosError, Result};
use std::{
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopAgentEventSink;

impl AgentEventSink for NoopAgentEventSink {
    fn emit(&self, _event: &AgentEvent) -> Result<()> {
        Ok(())
    }
}

static NOOP_AGENT_EVENT_SINK: NoopAgentEventSink = NoopAgentEventSink;

pub fn noop_agent_event_sink() -> &'static dyn AgentEventSink {
    &NOOP_AGENT_EVENT_SINK
}

#[derive(Clone)]
pub struct PersistingAgentEventSink {
    store: Arc<dyn SessionStore>,
    source: SessionSource,
    agent_id: Option<String>,
    workspace: Option<PathBuf>,
}

impl PersistingAgentEventSink {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self {
            store,
            source: SessionSource::Runtime,
            agent_id: None,
            workspace: None,
        }
    }

    pub fn with_source(mut self, source: SessionSource) -> Self {
        self.source = source;
        self
    }

    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    pub fn with_workspace(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }
}

impl AgentEventSink for PersistingAgentEventSink {
    fn emit(&self, event: &AgentEvent) -> Result<()> {
        let mut session = SessionRecord::new(event.session_id.clone(), self.source.clone());
        session.started_at = event.at;
        session.agent_id = self.agent_id.clone();
        session.workspace = self.workspace.clone();
        self.store.upsert_session(&session)?;
        self.store.append_agent_event(event)
    }

    fn emit_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        let mut session = SessionRecord::new(approval.session_id.clone(), self.source.clone());
        session.started_at = approval.at;
        session.agent_id = self.agent_id.clone();
        session.workspace = self.workspace.clone();
        self.store.upsert_session(&session)?;
        self.store.append_approval(approval)
    }
}

pub struct PersistingAgentTurnSink {
    store: Arc<dyn SessionStore>,
    source: SessionSource,
    agent_id: Option<String>,
    workspace: Option<PathBuf>,
    writer: Mutex<Option<Box<dyn SessionWriter>>>,
}

impl PersistingAgentTurnSink {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self {
            store,
            source: SessionSource::Runtime,
            agent_id: None,
            workspace: None,
            writer: Mutex::new(None),
        }
    }

    pub fn with_source(mut self, source: SessionSource) -> Self {
        self.source = source;
        self
    }

    pub fn with_agent_id(mut self, agent_id: impl Into<String>) -> Self {
        self.agent_id = Some(agent_id.into());
        self
    }

    pub fn with_workspace(mut self, workspace: impl Into<PathBuf>) -> Self {
        self.workspace = Some(workspace.into());
        self
    }

    pub fn commit(&self) -> Result<()> {
        let Some(writer) = self.lock_writer()?.take() else {
            return Ok(());
        };
        writer.commit()
    }

    pub fn rollback(&self) -> Result<()> {
        let Some(writer) = self.lock_writer()?.take() else {
            return Ok(());
        };
        writer.rollback()
    }

    pub fn append_entry(&self, entry: &SessionEntry) -> Result<()> {
        let turn_id = entry.turn_id.as_ref().ok_or_else(|| {
            IkarosError::Message("session turn sink requires entry turn_id".into())
        })?;
        let mut writer = self.lock_writer()?;
        self.ensure_writer(&mut writer, &entry.session_id, turn_id, entry.at)?;
        writer
            .as_mut()
            .expect("writer initialized")
            .append_entry(entry)
    }

    pub fn append_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        let turn_id = approval.turn_id.as_ref().ok_or_else(|| {
            IkarosError::Message("session turn sink requires approval turn_id".into())
        })?;
        let mut writer = self.lock_writer()?;
        self.ensure_writer(&mut writer, &approval.session_id, turn_id, approval.at)?;
        writer
            .as_mut()
            .expect("writer initialized")
            .append_approval(approval)
    }

    fn lock_writer(&self) -> Result<MutexGuard<'_, Option<Box<dyn SessionWriter>>>> {
        self.writer
            .lock()
            .map_err(|_| IkarosError::Message("session turn writer lock is poisoned".into()))
    }

    fn ensure_writer(
        &self,
        writer: &mut Option<Box<dyn SessionWriter>>,
        session_id: &SessionId,
        turn_id: &TurnId,
        started_at: time::OffsetDateTime,
    ) -> Result<()> {
        if writer.is_none() {
            let mut session = SessionRecord::new(session_id.clone(), self.source.clone());
            session.started_at = started_at;
            session.agent_id = self.agent_id.clone();
            session.workspace = self.workspace.clone();
            *writer = Some(self.store.begin_turn(&session, turn_id)?);
        }
        Ok(())
    }
}

impl AgentEventSink for PersistingAgentTurnSink {
    fn emit(&self, event: &AgentEvent) -> Result<()> {
        let mut writer = self.lock_writer()?;
        self.ensure_writer(&mut writer, &event.session_id, &event.turn_id, event.at)?;
        writer
            .as_mut()
            .expect("writer initialized")
            .append_agent_event(event)
    }

    fn emit_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        self.append_approval(approval)
    }
}
