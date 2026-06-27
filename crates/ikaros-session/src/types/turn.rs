// SPDX-License-Identifier: GPL-3.0-only

use super::{SessionId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTurnRecord {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub status: SessionTurnStatus,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub completed_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_owner: Option<String>,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub lease_expires_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SessionTurnRecord {
    pub fn new(session_id: impl Into<SessionId>, turn_id: impl Into<TurnId>) -> Self {
        let now = OffsetDateTime::now_utc();
        Self {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            status: SessionTurnStatus::Pending,
            started_at: now,
            updated_at: now,
            completed_at: None,
            lease_owner: None,
            lease_expires_at: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionTurnStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}
