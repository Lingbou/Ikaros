// SPDX-License-Identifier: GPL-3.0-only

use super::{AgentEvent, ApprovalRecord, SessionEntry, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTimelineItem {
    pub sequence: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub record: SessionTimelineRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionTimelineQuery {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub page: usize,
    pub page_size: usize,
}

impl SessionTimelineQuery {
    pub fn new(session_id: impl Into<SessionId>) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: None,
            page: 1,
            page_size: 50,
        }
    }

    pub fn for_turn(mut self, turn_id: impl Into<TurnId>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    pub fn with_page(mut self, page: usize, page_size: usize) -> Self {
        self.page = page.max(1);
        self.page_size = page_size.max(1);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTimelinePage {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    pub page: usize,
    pub page_size: usize,
    pub total_items: usize,
    pub items: Vec<SessionTimelineItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum SessionTimelineRecord {
    Entry(SessionEntry),
    AgentEvent(AgentEvent),
    Approval(ApprovalRecord),
}

impl SessionTimelineRecord {
    pub fn at(&self) -> OffsetDateTime {
        match self {
            Self::Entry(entry) => entry.at,
            Self::AgentEvent(event) => event.at,
            Self::Approval(approval) => approval.at,
        }
    }

    pub fn turn_id(&self) -> Option<TurnId> {
        match self {
            Self::Entry(entry) => entry.turn_id.clone(),
            Self::AgentEvent(event) => Some(event.turn_id.clone()),
            Self::Approval(approval) => approval.turn_id.clone(),
        }
    }
}
