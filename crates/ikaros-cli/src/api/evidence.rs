// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::api) struct ApiSessionEvidence {
    pub(in crate::api) sink: PersistingAgentTurnSink,
    pub(in crate::api) session_id: SessionId,
    pub(in crate::api) turn_id: TurnId,
}

impl ApiSessionEvidence {
    pub(in crate::api) fn new(agent: &AgentInstance, route: &str, model: &str) -> Result<Self> {
        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::new(&agent.state_dir));
        let sink = PersistingAgentTurnSink::new(store)
            .with_source(SessionSource::Service {
                name: "openai-compatible-api".into(),
            })
            .with_agent_id(agent.agent_id.clone())
            .with_workspace(agent.workspace.clone());
        let evidence = Self {
            sink,
            session_id,
            turn_id,
        };
        evidence.emit(
            AgentEventSource::Runtime,
            AgentEventKind::SessionStart,
            json!({
                "surface": "openai-compatible-api",
                "route": route,
                "model": model,
            }),
        )?;
        evidence.emit(
            AgentEventSource::Runtime,
            AgentEventKind::TurnStart,
            json!({
                "surface": "openai-compatible-api",
                "route": route,
                "model": model,
            }),
        )?;
        Ok(evidence)
    }

    pub(in crate::api) fn ids(&self) -> ApiSessionIds {
        ApiSessionIds {
            session_id: self.session_id.as_str().to_owned(),
            turn_id: self.turn_id.as_str().to_owned(),
        }
    }

    pub(in crate::api) fn append_entry(
        &self,
        kind: SessionEntryKind,
        visible_text: Option<String>,
        payload: Value,
    ) -> Result<()> {
        let mut entry = SessionEntry::new(self.session_id.clone(), kind);
        entry.turn_id = Some(self.turn_id.clone());
        entry.visible_text = visible_text;
        entry.payload = payload;
        Ok(self.sink.append_entry(&entry)?)
    }

    pub(in crate::api) fn emit(
        &self,
        source: AgentEventSource,
        kind: AgentEventKind,
        payload: Value,
    ) -> Result<()> {
        Ok(self.sink.emit(&AgentEvent::new(
            self.session_id.clone(),
            self.turn_id.clone(),
            None,
            source,
            kind,
            payload,
        ))?)
    }

    pub(in crate::api) fn commit(self) -> Result<()> {
        Ok(self.sink.commit()?)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::api) struct ApiSessionIds {
    pub(in crate::api) session_id: String,
    pub(in crate::api) turn_id: String,
}

impl ApiSessionIds {
    pub(in crate::api) fn correlation_id(&self) -> String {
        format!("session:{}:turn:{}", self.session_id, self.turn_id)
    }
}

pub(in crate::api) fn api_text_preview(value: &str) -> String {
    let redacted = redact_secrets(value);
    let mut preview = redacted.chars().take(512).collect::<String>();
    if redacted.chars().count() > 512 {
        preview.push_str("...");
    }
    preview
}
