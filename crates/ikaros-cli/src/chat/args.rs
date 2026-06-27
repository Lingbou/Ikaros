// SPDX-License-Identifier: GPL-3.0-only

use clap::Args;
use ikaros_agent::chat::{ChatRunOptions, DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET};

use super::attachments;

#[derive(Debug, Args, Clone)]
pub(crate) struct ChatArgs {
    #[arg(long)]
    pub(in crate::chat) message: Option<String>,
    #[arg(long = "chat-session")]
    pub(in crate::chat) chat_session: Option<String>,
    #[arg(long)]
    pub(in crate::chat) stream: bool,
    #[arg(long = "image", value_name = "URL_OR_PATH")]
    pub(in crate::chat) image: Vec<String>,
    #[arg(long = "audio", value_name = "URL_OR_PATH")]
    pub(in crate::chat) audio: Vec<String>,
    #[arg(long = "file", value_name = "URL_OR_PATH")]
    pub(in crate::chat) file: Vec<String>,
    #[arg(long = "no-agent-loop")]
    pub(in crate::chat) no_agent_loop: bool,
    #[arg(long, default_value_t = 3)]
    pub(in crate::chat) memory_limit: usize,
    #[arg(long = "memory-search-limit", default_value_t = 0)]
    pub(in crate::chat) memory_search_limit: usize,
    #[arg(long, default_value_t = 0)]
    pub(in crate::chat) rag_top_k: usize,
    #[arg(long, default_value_t = 3)]
    pub(in crate::chat) history_context_limit: usize,
    #[arg(long, default_value_t = 12)]
    pub(in crate::chat) history_summary_limit: usize,
    #[arg(long = "context-token-budget", default_value_t = DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET)]
    pub(in crate::chat) context_token_budget: usize,
    #[arg(long = "context-engine")]
    pub(in crate::chat) context_engine: Option<String>,
    #[arg(long = "no-relationship-learning")]
    pub(in crate::chat) no_relationship_learning: bool,
    #[arg(long)]
    pub(in crate::chat) scope: Option<String>,
    #[arg(long)]
    pub(in crate::chat) no_context: bool,
    #[arg(long)]
    pub(in crate::chat) history: bool,
    #[arg(long)]
    pub(in crate::chat) sessions: bool,
    #[arg(long, default_value_t = 20)]
    pub(in crate::chat) history_limit: usize,
    #[arg(long = "history-session")]
    pub(in crate::chat) history_session: Option<String>,
    #[arg(long = "history-search")]
    pub(in crate::chat) history_search: Option<String>,
    #[arg(long = "history-delete-session")]
    pub(in crate::chat) history_delete_session: Option<String>,
    #[arg(long = "history-clear")]
    pub(in crate::chat) history_clear: bool,
}

impl Default for ChatArgs {
    fn default() -> Self {
        Self {
            message: None,
            chat_session: None,
            stream: false,
            image: Vec::new(),
            audio: Vec::new(),
            file: Vec::new(),
            no_agent_loop: false,
            memory_limit: 3,
            memory_search_limit: 0,
            rag_top_k: 0,
            history_context_limit: 3,
            history_summary_limit: 12,
            context_token_budget: DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET,
            context_engine: None,
            no_relationship_learning: false,
            scope: None,
            no_context: false,
            history: false,
            sessions: false,
            history_limit: 20,
            history_session: None,
            history_search: None,
            history_delete_session: None,
            history_clear: false,
        }
    }
}

impl From<&ChatArgs> for ChatRunOptions {
    fn from(args: &ChatArgs) -> Self {
        Self {
            stream: args.stream,
            agent_loop: !args.no_agent_loop,
            memory_limit: args.memory_limit,
            memory_search_limit: args.memory_search_limit,
            rag_top_k: args.rag_top_k,
            history_context_limit: args.history_context_limit,
            history_summary_limit: args.history_summary_limit,
            context_token_budget: args.context_token_budget,
            context_engine: args.context_engine.clone(),
            relationship_learning: !args.no_relationship_learning,
            scope: args.scope.clone(),
            no_context: args.no_context,
            session_id: args.chat_session.clone(),
            turn_id: None,
            session_source: None,
            session_state_db: None,
            safe_tools: false,
            content_blocks: attachments::content_blocks_from_args(
                &args.image,
                &args.audio,
                &args.file,
            ),
            cancellation: Default::default(),
        }
    }
}
