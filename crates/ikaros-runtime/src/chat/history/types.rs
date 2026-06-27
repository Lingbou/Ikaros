// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatHistoryRecord {
    pub session_id: String,
    pub turn_id: String,
    pub created_at: String,
    pub agent: String,
    pub provider: String,
    pub model: String,
    pub streamed: bool,
    pub user_message: String,
    pub assistant_message: String,
    pub relationship_hits: usize,
    pub memory_hits: usize,
    pub rag_hits: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatHistorySessionSummary {
    pub session_id: String,
    pub turns: usize,
    pub first_created_at: String,
    pub last_created_at: String,
    pub last_turn_id: String,
    pub agents: Vec<String>,
    pub providers: Vec<String>,
    pub models: Vec<String>,
    pub last_user_message: String,
    pub last_assistant_message: String,
}
