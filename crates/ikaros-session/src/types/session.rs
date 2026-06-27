// SPDX-License-Identifier: GPL-3.0-only

use super::{SessionEntryId, SessionId};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use time::OffsetDateTime;

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
