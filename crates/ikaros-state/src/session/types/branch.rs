// SPDX-License-Identifier: GPL-3.0-only

use super::{SessionEntry, SessionEntryId, SessionId, SessionRecord};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionBranch {
    pub session: SessionRecord,
    pub entries: Vec<SessionEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionBranchSummaryInput {
    pub session_id: SessionId,
    pub parent_entry_id: SessionEntryId,
    pub summary: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionCompactionInput {
    pub session_id: SessionId,
    pub parent_entry_id: SessionEntryId,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compacted_entry_ids: Vec<SessionEntryId>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionRetryInput {
    pub session_id: SessionId,
    pub parent_entry_id: SessionEntryId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}
