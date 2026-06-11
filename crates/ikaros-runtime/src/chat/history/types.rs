// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ChatHistoryBackend {
    Jsonl,
    Sqlite,
}

impl ChatHistoryBackend {
    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "jsonl" => Ok(Self::Jsonl),
            "sqlite" => Ok(Self::Sqlite),
            other => Err(IkarosError::Message(format!(
                "unsupported chat history backend: {other}"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Jsonl => "jsonl",
            Self::Sqlite => "sqlite",
        }
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatHistoryStore {
    pub(super) path: PathBuf,
    pub(super) backend: ChatHistoryBackend,
}

#[derive(Debug, Clone)]
pub struct ChatHistoryAppend<'a> {
    pub session_id: &'a str,
    pub agent: &'a str,
    pub provider: &'a str,
    pub model: &'a str,
    pub streamed: bool,
    pub user_message: &'a str,
    pub assistant_message: &'a str,
    pub relationship_hits: usize,
    pub memory_hits: usize,
    pub rag_hits: usize,
}
