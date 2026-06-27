// SPDX-License-Identifier: GPL-3.0-only

use super::{SessionId, SessionInputId, TurnId};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionInput {
    pub input_id: SessionInputId,
    pub session_id: SessionId,
    pub status: SessionInputStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key_digest: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
    #[serde(with = "time::serde::rfc3339")]
    pub admitted_at: OffsetDateTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub promoted_turn_id: Option<TurnId>,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub promoted_at: Option<OffsetDateTime>,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub cancelled_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel_reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionInputAdmission {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key_digest: Option<String>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

impl SessionInputAdmission {
    pub fn new(session_id: impl Into<SessionId>, payload: serde_json::Value) -> Self {
        Self {
            session_id: session_id.into(),
            idempotency_key_digest: None,
            payload,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionInputStatus {
    Admitted,
    Promoted,
    Cancelled,
}
