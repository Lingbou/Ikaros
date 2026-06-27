// SPDX-License-Identifier: GPL-3.0-only

use super::{SessionEntry, SessionId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionSearchQuery {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    pub limit: usize,
}

impl SessionSearchQuery {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            session_id: None,
            limit: 20,
        }
    }

    pub fn for_session(mut self, session_id: impl Into<SessionId>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionSearchIndex {
    Fts,
    Trigram,
    Substring,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionSearchHit {
    pub entry: SessionEntry,
    pub snippet: String,
    pub score: f64,
    pub index: SessionSearchIndex,
}
