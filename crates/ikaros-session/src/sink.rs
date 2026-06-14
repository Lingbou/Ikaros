// SPDX-License-Identifier: GPL-3.0-only

use crate::{AgentEvent, AgentEventSink, SessionRecord, SessionSource, SessionStore};
use ikaros_core::Result;
use std::{path::PathBuf, sync::Arc};

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
}
