// SPDX-License-Identifier: GPL-3.0-only

mod context;
mod context_engine;
mod history;
mod learning;
mod prompt;
mod turn;
mod types;

pub use context::{context_lookup_is_safe_read, extract_memory_context, extract_rag_context};
pub use context_engine::{
    CompactInput, CompactReport, ContextAssembleInput, ContextBundle, ContextEngine, ContextEvent,
    LocalChatContextEngine, TurnRecord, build_chat_context, build_chat_context_bundle_with_engine,
    build_chat_context_with_engine,
};
pub use history::{
    ChatHistoryRecord, ChatHistorySessionSummary, ChatHistoryStore, new_chat_session_id,
};
pub use prompt::{render_chat_system_prompt, render_persona_agent_context};
pub use turn::{ChatTurnEventOptions, run_chat_message, run_chat_turn, run_chat_turn_with_events};
pub use types::{
    ChatContext, ChatMessageResult, ChatRunOptions, ChatTurnReport,
    DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET,
};

#[cfg(test)]
mod tests;
