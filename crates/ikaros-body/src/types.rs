// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{PolicyDecision, TaskState, redact_secrets};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BodyKind {
    Cli,
    Desktop,
    Live2D,
    Vrm,
    Voice,
    Web,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum BodyEventKind {
    Status,
    Emotion,
    Task,
    Plan,
    Skill,
    Memory,
    Rag,
    Approval,
    Audit,
    Message,
    Error,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyContextSources {
    pub memory: Vec<String>,
    pub rag: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyEvent {
    pub body: BodyKind,
    pub kind: BodyEventKind,
    pub message: String,
    pub data: BTreeMap<String, String>,
}

impl BodyEvent {
    pub fn new(
        body: BodyKind,
        kind: BodyEventKind,
        message: impl Into<String>,
        data: BTreeMap<String, String>,
    ) -> Self {
        Self {
            body,
            kind,
            message: redact_secrets(&message.into()),
            data: data
                .into_iter()
                .map(|(key, value)| (key, redact_secrets(&value)))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyStatus {
    pub persona_name: String,
    pub emotion: String,
    pub task_id: Option<String>,
    pub task_state: Option<TaskState>,
    pub context_sources: BodyContextSources,
    pub policy_decisions: Vec<PolicyDecision>,
    pub audit_path: Option<PathBuf>,
    pub approvals_path: Option<PathBuf>,
}

impl BodyStatus {
    pub fn new(persona_name: impl Into<String>, emotion: impl Into<String>) -> Self {
        Self {
            persona_name: redact_secrets(&persona_name.into()),
            emotion: emotion.into(),
            task_id: None,
            task_state: None,
            context_sources: BodyContextSources::default(),
            policy_decisions: Vec::new(),
            audit_path: None,
            approvals_path: None,
        }
    }

    pub fn with_task(
        mut self,
        task_id: impl Into<String>,
        task_state: impl Into<Option<TaskState>>,
    ) -> Self {
        self.task_id = Some(redact_secrets(&task_id.into()));
        self.task_state = task_state.into();
        self
    }

    pub fn with_context_sources(mut self, memory: Vec<String>, rag: Vec<String>) -> Self {
        self.context_sources = BodyContextSources {
            memory: memory
                .into_iter()
                .map(|item| redact_secrets(&item))
                .collect(),
            rag: rag.into_iter().map(|item| redact_secrets(&item)).collect(),
        };
        self
    }

    pub fn with_policy_decisions(mut self, policy_decisions: Vec<PolicyDecision>) -> Self {
        self.policy_decisions = policy_decisions;
        self
    }

    pub fn with_audit_path(mut self, audit_path: impl Into<PathBuf>) -> Self {
        self.audit_path = Some(PathBuf::from(redact_secrets(
            &audit_path.into().display().to_string(),
        )));
        self
    }

    pub fn with_approvals_path(mut self, approvals_path: impl Into<PathBuf>) -> Self {
        self.approvals_path = Some(PathBuf::from(redact_secrets(
            &approvals_path.into().display().to_string(),
        )));
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BodyFrame {
    pub body: BodyKind,
    pub status: BodyStatus,
    pub events: Vec<BodyEvent>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_events_redact_secret_like_data() {
        let event = BodyEvent::new(
            BodyKind::Cli,
            BodyEventKind::Message,
            "received sk-not-real",
            BTreeMap::from([("token".into(), "api_key=abc".into())]),
        );
        let json = serde_json::to_string(&event).expect("json");
        assert!(!json.contains("sk-not-real"));
        assert!(!json.contains("abc"));
        assert!(json.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn body_status_redacts_secret_like_paths() {
        let status = BodyStatus::new("Ikaros", "Neutral")
            .with_audit_path("/tmp/token=abc123/audit.jsonl")
            .with_approvals_path("/tmp/sk-not-real/approvals.jsonl");
        let json = serde_json::to_string(&status).expect("json");
        assert!(!json.contains("abc123"));
        assert!(!json.contains("sk-not-real"));
        assert!(json.contains("[REDACTED_SECRET]"));
    }
}
