// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::{AgentEvent, AgentLoopOptions, AgentLoopReport};
use ikaros_session::{SessionContinuation, SessionEntry, SessionId, TurnId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentHarnessConfig {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub system_prompt: String,
    pub options: AgentLoopOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentHarnessMessage {
    pub content: String,
}

impl AgentHarnessMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentHarnessPendingCounts {
    pub steer: usize,
    pub follow_up: usize,
    pub next_turn: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentHarnessTurn {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub events: Vec<AgentEvent>,
    pub report: AgentLoopReport,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentHarnessContinuation {
    Turn(AgentHarnessTurn),
    Entry {
        continuation: SessionContinuation,
        entry: SessionEntry,
    },
    ToolResult {
        continuation: SessionContinuation,
        turn_id: TurnId,
        tool_name: String,
        result: serde_json::Value,
    },
}
