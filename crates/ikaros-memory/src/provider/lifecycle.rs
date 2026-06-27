// SPDX-License-Identifier: GPL-3.0-only

use crate::{MemoryKind, MemoryQuery, MemoryRecord, MemoryRef};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryLifecycleReport {
    pub phase: String,
    pub records_read: usize,
    pub records_written: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<MemoryRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<MemoryLifecycleRecordRef>,
    pub notes: Vec<String>,
}

impl MemoryLifecycleReport {
    pub fn noop(phase: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            records_read: 0,
            records_written: 0,
            source_ref: None,
            records: Vec::new(),
            notes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryLifecycleRecordRef {
    pub id: String,
    pub kind: MemoryKind,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<MemoryRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

impl From<&MemoryRecord> for MemoryLifecycleRecordRef {
    fn from(record: &MemoryRecord) -> Self {
        Self {
            id: record.id.clone(),
            kind: record.kind.clone(),
            scope: record.scope.clone(),
            source_ref: record.source_ref.clone(),
            confidence: record.confidence,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryTurnStart {
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub user_input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryPrefetchInput {
    pub query: MemoryQuery,
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryTurnRecord {
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub agent_id: Option<String>,
    pub user_input: String,
    pub assistant_output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryPreCompressInput {
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub budget_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemorySessionSwitch {
    pub from_session_id: Option<String>,
    pub to_session_id: Option<String>,
    pub agent_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryDelegationObservation {
    pub parent_agent_id: Option<String>,
    pub child_agent_id: Option<String>,
    pub summary: String,
}
