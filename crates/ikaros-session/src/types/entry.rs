// SPDX-License-Identifier: GPL-3.0-only

use super::{SessionEntryId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionEntry {
    pub entry_id: SessionEntryId,
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_entry_id: Option<SessionEntryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub kind: SessionEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visible_text: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

impl SessionEntry {
    pub fn new(session_id: impl Into<SessionId>, kind: SessionEntryKind) -> Self {
        Self {
            entry_id: SessionEntryId::new(),
            session_id: session_id.into(),
            parent_entry_id: None,
            turn_id: None,
            at: OffsetDateTime::now_utc(),
            kind,
            visible_text: None,
            payload: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionEntryKind {
    SystemMessage,
    UserMessage,
    AssistantMessage,
    ToolResult,
    ModelChange,
    Compaction,
    BranchSummary,
    Custom,
    Leaf,
}
