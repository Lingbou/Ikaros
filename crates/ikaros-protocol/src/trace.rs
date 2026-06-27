// SPDX-License-Identifier: GPL-3.0-only

use crate::envelope::IKAROS_PROTOCOL_VERSION;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Pending,
    Running,
    WaitingApproval,
    WaitingContinuation,
    RunningTool,
    Compacting,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateTraceEntry {
    pub protocol_version: u32,
    pub session_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub correlation_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub source: String,
    pub category: String,
    pub event_kind: String,
    pub state_before: TurnStatus,
    pub state_after: TurnStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
}

impl StateTraceEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        event_id: impl Into<String>,
        at: OffsetDateTime,
        source: impl Into<String>,
        category: impl Into<String>,
        event_kind: impl Into<String>,
        state_before: TurnStatus,
        state_after: TurnStatus,
        payload: Value,
    ) -> Self {
        let session_id = session_id.into();
        let turn_id = turn_id.into();
        Self {
            protocol_version: IKAROS_PROTOCOL_VERSION,
            correlation_id: turn_correlation_id(&session_id, &turn_id),
            session_id,
            turn_id,
            event_id: event_id.into(),
            at,
            source: source.into(),
            category: category.into(),
            event_kind: event_kind.into(),
            state_before,
            state_after,
            title: None,
            detail: None,
            waiting_on: None,
            stop_reason: None,
            error: None,
            payload,
        }
    }
}

pub fn turn_correlation_id(session_id: &str, turn_id: &str) -> String {
    format!("session:{session_id}:turn:{turn_id}")
}
