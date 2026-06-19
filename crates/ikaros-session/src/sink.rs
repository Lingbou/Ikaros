// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AgentEvent, AgentEventSink, ApprovalRecord, SessionEntry, SessionId, SessionRecord,
    SessionSource, SessionStore, TurnId,
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
    buffer: Mutex<Option<BufferedTurn>>,
}

struct BufferedTurn {
    session_id: SessionId,
    turn_id: TurnId,
    started_at: time::OffsetDateTime,
    items: Vec<BufferedTurnItem>,
}

enum BufferedTurnItem {
    Entry(SessionEntry),
    AgentEvent(AgentEvent),
    Approval(ApprovalRecord),
}

impl PersistingAgentTurnSink {
    pub fn new(store: Arc<dyn SessionStore>) -> Self {
        Self {
            store,
            source: SessionSource::Runtime,
            agent_id: None,
            workspace: None,
            buffer: Mutex::new(None),
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
        let Some(buffer) = self.lock_buffer()?.take() else {
            return Ok(());
        };
        let mut session = SessionRecord::new(buffer.session_id.clone(), self.source.clone());
        session.started_at = buffer.started_at;
        session.agent_id = self.agent_id.clone();
        session.workspace = self.workspace.clone();
        let mut writer = self.store.begin_turn(&session, &buffer.turn_id)?;
        for item in buffer.items {
            match item {
                BufferedTurnItem::Entry(entry) => writer.append_entry(&entry)?,
                BufferedTurnItem::AgentEvent(event) => writer.append_agent_event(&event)?,
                BufferedTurnItem::Approval(approval) => writer.append_approval(&approval)?,
            }
        }
        writer.commit()
    }

    pub fn rollback(&self) -> Result<()> {
        let _ = self.lock_buffer()?.take();
        Ok(())
    }

    pub fn append_entry(&self, entry: &SessionEntry) -> Result<()> {
        let turn_id = entry.turn_id.as_ref().ok_or_else(|| {
            IkarosError::Message("session turn sink requires entry turn_id".into())
        })?;
        let mut buffer = self.lock_buffer()?;
        self.ensure_buffer(&mut buffer, &entry.session_id, turn_id, entry.at)?;
        buffer
            .as_mut()
            .expect("buffer initialized")
            .items
            .push(BufferedTurnItem::Entry(entry.clone()));
        Ok(())
    }

    pub fn append_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        let turn_id = approval.turn_id.as_ref().ok_or_else(|| {
            IkarosError::Message("session turn sink requires approval turn_id".into())
        })?;
        let mut buffer = self.lock_buffer()?;
        self.ensure_buffer(&mut buffer, &approval.session_id, turn_id, approval.at)?;
        buffer
            .as_mut()
            .expect("buffer initialized")
            .items
            .push(BufferedTurnItem::Approval(approval.clone()));
        Ok(())
    }

    fn lock_buffer(&self) -> Result<MutexGuard<'_, Option<BufferedTurn>>> {
        self.buffer
            .lock()
            .map_err(|_| IkarosError::Message("session turn buffer lock is poisoned".into()))
    }

    fn ensure_buffer(
        &self,
        buffer: &mut Option<BufferedTurn>,
        session_id: &SessionId,
        turn_id: &TurnId,
        started_at: time::OffsetDateTime,
    ) -> Result<()> {
        if let Some(buffer) = buffer.as_ref() {
            if &buffer.session_id != session_id {
                return Err(IkarosError::Message(format!(
                    "session turn sink expected session {}, got {}",
                    buffer.session_id, session_id
                )));
            }
            if &buffer.turn_id != turn_id {
                return Err(IkarosError::Message(format!(
                    "session turn sink expected turn {}, got {}",
                    buffer.turn_id, turn_id
                )));
            }
            return Ok(());
        }
        *buffer = Some(BufferedTurn {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
            started_at,
            items: Vec::new(),
        });
        Ok(())
    }
}

impl AgentEventSink for PersistingAgentTurnSink {
    fn emit(&self, event: &AgentEvent) -> Result<()> {
        let mut buffer = self.lock_buffer()?;
        self.ensure_buffer(&mut buffer, &event.session_id, &event.turn_id, event.at)?;
        buffer
            .as_mut()
            .expect("buffer initialized")
            .items
            .push(BufferedTurnItem::AgentEvent(event.clone()));
        Ok(())
    }

    fn emit_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        self.append_approval(approval)
    }
}
