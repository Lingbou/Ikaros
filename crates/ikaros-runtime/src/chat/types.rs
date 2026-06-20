// SPDX-License-Identifier: GPL-3.0-only

pub use ikaros_context::{ChatContext, DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET};
use ikaros_harness::CancellationToken;
use ikaros_models::ModelResponse;
use ikaros_session::SessionSource;
use ikaros_soul::EmotionState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRunOptions {
    pub stream: bool,
    pub agent_loop: bool,
    pub memory_limit: usize,
    pub rag_top_k: usize,
    pub history_context_limit: usize,
    pub history_summary_limit: usize,
    pub context_token_budget: usize,
    pub relationship_learning: bool,
    pub scope: Option<String>,
    pub no_context: bool,
    pub session_id: Option<String>,
    pub session_source: Option<SessionSource>,
    pub chat_history_path: Option<PathBuf>,
    pub chat_history_backend: Option<String>,
    #[serde(skip)]
    pub cancellation: CancellationToken,
}

impl Default for ChatRunOptions {
    fn default() -> Self {
        Self {
            stream: false,
            agent_loop: true,
            memory_limit: 3,
            rag_top_k: 0,
            history_context_limit: 3,
            history_summary_limit: 12,
            context_token_budget: DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET,
            relationship_learning: true,
            scope: None,
            no_context: false,
            session_id: None,
            session_source: None,
            chat_history_path: None,
            chat_history_backend: None,
            cancellation: CancellationToken::new(),
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
    pub relationship_candidates_created: usize,
    pub reference_hits: usize,
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
    pub relationship_candidates_created: usize,
    pub reference_hits: usize,
    pub history_hits: usize,
    pub memory_hits: usize,
    pub rag_hits: usize,
    pub chat_history_path: Option<PathBuf>,
    pub chat_session_id: Option<String>,
}
