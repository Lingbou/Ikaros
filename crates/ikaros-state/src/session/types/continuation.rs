// SPDX-License-Identifier: GPL-3.0-only

use super::{ContinuationId, SessionId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionContinuation {
    pub continuation_id: ContinuationId,
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_continuation_id: Option<ContinuationId>,
    pub kind: SessionContinuationKind,
    pub status: SessionContinuationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<SessionContinuationStatusReason>,
    pub priority: i64,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub claimed_at: Option<OffsetDateTime>,
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
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub attempt_count: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionContinuationInput {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_continuation_id: Option<ContinuationId>,
    pub kind: SessionContinuationKind,
    pub priority: i64,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

impl SessionContinuationInput {
    pub fn new(session_id: impl Into<SessionId>, kind: SessionContinuationKind) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: None,
            parent_continuation_id: None,
            kind,
            priority: kind.default_priority(),
            payload: serde_json::Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SessionContinuationClaim {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<SessionContinuationKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_owner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_duration_seconds: Option<i64>,
}

impl SessionContinuationClaim {
    pub fn for_session(session_id: impl Into<SessionId>) -> Self {
        Self {
            session_id: Some(session_id.into()),
            turn_id: None,
            kinds: Vec::new(),
            lease_owner: None,
            lease_duration_seconds: None,
        }
    }

    pub fn with_turn(mut self, turn_id: impl Into<TurnId>) -> Self {
        self.turn_id = Some(turn_id.into());
        self
    }

    pub fn with_kinds(mut self, kinds: impl IntoIterator<Item = SessionContinuationKind>) -> Self {
        self.kinds = kinds.into_iter().collect();
        self
    }

    pub fn with_lease_owner(mut self, lease_owner: impl Into<String>) -> Self {
        self.lease_owner = Some(lease_owner.into());
        self
    }

    pub fn with_lease_duration_seconds(mut self, seconds: i64) -> Self {
        self.lease_duration_seconds = Some(seconds);
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SessionContinuationKind {
    Steer,
    FollowUp,
    NextTurn,
    Resume,
    Retry,
    Compact,
    ToolResult,
}

impl SessionContinuationKind {
    pub fn default_priority(self) -> i64 {
        match self {
            Self::Steer => 0,
            Self::FollowUp => 10,
            Self::NextTurn => 20,
            Self::Resume => 30,
            Self::Compact => 40,
            Self::Retry => 50,
            Self::ToolResult => 55,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionContinuationStatus {
    Queued,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionContinuationStatusReason {
    Enqueued,
    Claimed,
    Completed,
    Failed,
    Cancelled,
    Requeued,
    LeaseExpired,
}

fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}
