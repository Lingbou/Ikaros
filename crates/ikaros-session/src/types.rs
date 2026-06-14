// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::Result;
use ikaros_models::ModelStreamEvent;
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};
use time::OffsetDateTime;
use uuid::Uuid;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4().to_string())
            }

            pub fn from_string(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

string_id!(SessionId);
string_id!(TurnId);
string_id!(EventId);
string_id!(SessionEntryId);

pub type AgentEventId = EventId;
pub type AgentSessionId = SessionId;
pub type AgentTurnId = TurnId;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionRecord {
    pub session_id: SessionId,
    pub source: SessionSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_leaf_entry_id: Option<SessionEntryId>,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub ended_at: Option<OffsetDateTime>,
}

impl SessionRecord {
    pub fn new(session_id: impl Into<SessionId>, source: SessionSource) -> Self {
        Self {
            session_id: session_id.into(),
            source,
            agent_id: None,
            workspace: None,
            parent_session_id: None,
            active_leaf_entry_id: None,
            started_at: OffsetDateTime::now_utc(),
            ended_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SessionSource {
    Cli,
    Gateway {
        channel: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        peer: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    Schedule {
        job_id: String,
    },
    Subagent {
        parent_agent_id: String,
    },
    Service {
        name: String,
    },
    Runtime,
    Test,
}

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentEvent {
    pub event_id: AgentEventId,
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<AgentEventId>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub source: AgentEventSource,
    pub kind: AgentEventKind,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

impl AgentEvent {
    pub fn new(
        session_id: impl Into<AgentSessionId>,
        turn_id: impl Into<AgentTurnId>,
        parent_event_id: Option<AgentEventId>,
        source: AgentEventSource,
        kind: AgentEventKind,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            event_id: EventId::new(),
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            parent_event_id,
            at: OffsetDateTime::now_utc(),
            source,
            kind,
            payload,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventSource {
    Runtime,
    User,
    Model,
    Tool,
    Harness,
    Context,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentEventKind {
    SessionStart,
    TurnStart,
    UserMessage,
    ModelStream(ModelStreamEvent),
    ToolStart,
    ToolUpdate,
    ToolEnd,
    ContextCompacted,
    ApprovalRequested,
    ApprovalResolved,
    TurnEnd,
    Error,
}

pub trait AgentEventSink: Send + Sync {
    fn emit(&self, event: &AgentEvent) -> Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ApprovalRecord {
    pub approval_id: String,
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub status: ApprovalStatus,
    pub request: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Requested,
    Approved,
    Denied,
    Expired,
    Executed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionReplay {
    pub session: SessionRecord,
    pub entries: Vec<SessionEntry>,
    pub agent_events: Vec<AgentEvent>,
    pub approvals: Vec<ApprovalRecord>,
}
