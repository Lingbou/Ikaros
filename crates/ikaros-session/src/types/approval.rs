// SPDX-License-Identifier: GPL-3.0-only

use super::{SessionId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

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
