// SPDX-License-Identifier: GPL-3.0-only

mod context;
mod context_engine;
mod history;
mod learning;
mod prompt;
mod session_runtime;
mod turn;
mod types;

pub use context::{
    context_lookup_is_safe_read, extract_rag_context, extract_retrieved_memory_context,
};
pub use context_engine::{
    CompactInput, CompactReport, ContextAssembleInput, ContextBundle, ContextEngine, ContextEvent,
    ContextModelBudget, LocalChatContextEngine, TurnRecord, build_chat_context,
    build_chat_context_bundle_with_engine, build_chat_context_bundle_with_model_context,
    build_chat_context_with_engine,
};
pub use history::{
    CHAT_HISTORY_DELETE_SESSION_OPERATION, ChatHistoryRecord, ChatHistorySessionSummary,
    chat_history_records_from_session_replay, chat_history_session_summaries_from_session_replays,
    new_chat_session_id, search_chat_history_records,
};
pub use prompt::{render_chat_system_prompt, render_persona_agent_context};
pub use session_runtime::{
    ChatSessionRuntime, ChatSessionRuntimeConfig, ChatSessionRuntimeEvent,
    ChatSessionRuntimeEventReceiver, ChatSessionRuntimeHandle, ChatSessionTurnResult,
};
pub use turn::{
    ChatTurnEventOptions, apply_chat_memory_policy, chat_memory_policy_from_config,
    emit_chat_memory_lifecycle_report, run_chat_message, run_chat_turn, run_chat_turn_with_events,
};
pub use types::{
    ChatContext, ChatMessageResult, ChatRunOptions, ChatTurnReport,
    DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET,
};

#[cfg(test)]
mod tests;
