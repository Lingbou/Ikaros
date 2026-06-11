// SPDX-License-Identifier: GPL-3.0-only

use ikaros_models::ModelResponse;
use ikaros_soul::EmotionState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const DEFAULT_CHAT_CONTEXT_CHAR_BUDGET: usize = 8_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRunOptions {
    pub stream: bool,
    pub agent_loop: bool,
    pub memory_limit: usize,
    pub rag_top_k: usize,
    pub history_context_limit: usize,
    pub history_summary_limit: usize,
    pub context_char_budget: usize,
    pub relationship_learning: bool,
    pub scope: Option<String>,
    pub no_context: bool,
    pub session_id: Option<String>,
    pub chat_history_path: Option<PathBuf>,
    pub chat_history_backend: Option<String>,
}

impl Default for ChatRunOptions {
    fn default() -> Self {
        Self {
            stream: false,
            agent_loop: true,
            memory_limit: 3,
            rag_top_k: 3,
            history_context_limit: 3,
            history_summary_limit: 12,
            context_char_budget: DEFAULT_CHAT_CONTEXT_CHAR_BUDGET,
            relationship_learning: true,
            scope: None,
            no_context: false,
            session_id: None,
            chat_history_path: None,
            chat_history_backend: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessageResult {
    pub content: String,
    pub provider: String,
    pub model: String,
    pub emotion: EmotionState,
    pub streamed: bool,
    pub stream_chunks: Vec<String>,
    pub relationship_hits: usize,
    pub relationship_learned: usize,
    pub history_hits: usize,
    pub memory_hits: usize,
    pub rag_hits: usize,
    pub audit_path: PathBuf,
    pub model_usage_path: PathBuf,
    pub chat_history_path: PathBuf,
    pub chat_session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatTurnReport {
    pub response: ModelResponse,
    pub emotion: EmotionState,
    pub streamed: bool,
    pub stream_chunks: Vec<String>,
    pub relationship_hits: usize,
    pub relationship_learned: usize,
    pub history_hits: usize,
    pub memory_hits: usize,
    pub rag_hits: usize,
    pub chat_history_path: Option<PathBuf>,
    pub chat_session_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChatContext {
    pub relationship: Vec<String>,
    pub history: Vec<String>,
    pub memory: Vec<String>,
    pub rag: Vec<String>,
}
