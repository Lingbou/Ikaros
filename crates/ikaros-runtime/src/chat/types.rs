// SPDX-License-Identifier: GPL-3.0-only

pub use ikaros_context::{ChatContext, DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET};
use ikaros_harness::CancellationToken;
use ikaros_models::{ModelContentBlock, ModelResponse};
use ikaros_session::SessionSource;
use ikaros_soul::EmotionState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRunOptions {
    pub stream: bool,
    pub agent_loop: bool,
    pub memory_limit: usize,
    pub memory_search_limit: usize,
    pub rag_top_k: usize,
    pub history_context_limit: usize,
    pub history_summary_limit: usize,
    pub context_token_budget: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_engine: Option<String>,
    pub relationship_learning: bool,
    pub scope: Option<String>,
    pub no_context: bool,
    pub session_id: Option<String>,
    pub turn_id: Option<String>,
    pub session_source: Option<SessionSource>,
    pub session_state_db: Option<PathBuf>,
    pub safe_tools: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_blocks: Vec<ModelContentBlock>,
    #[serde(skip)]
    pub cancellation: CancellationToken,
}

impl Default for ChatRunOptions {
    fn default() -> Self {
        Self {
            stream: false,
            agent_loop: true,
            memory_limit: 3,
            memory_search_limit: 0,
            rag_top_k: 0,
            history_context_limit: 3,
            history_summary_limit: 12,
            context_token_budget: DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET,
            context_engine: None,
            relationship_learning: true,
            scope: None,
            no_context: false,
            session_id: None,
            turn_id: None,
            session_source: None,
            session_state_db: None,
            safe_tools: false,
            content_blocks: Vec::new(),
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
    pub session_state_db: PathBuf,
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
    pub chat_session_id: Option<String>,
}
