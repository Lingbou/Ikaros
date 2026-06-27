// SPDX-License-Identifier: GPL-3.0-only

pub mod attachments;
mod workbench_diff;
pub mod workbench_state;

mod context;
mod context_engine;
mod history;
mod learning;
mod prompt;
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
    append_chat_history_delete_tombstone, chat_history_records_from_session_replay,
    chat_history_session_summaries_from_session_replays, new_chat_session_id,
    search_chat_history_records,
};
pub use prompt::{render_chat_system_prompt, render_persona_agent_context};
pub use turn::{
    ChatMessageContext, ChatTurnEventOptions, apply_chat_memory_policy,
    chat_memory_policy_from_config, emit_chat_memory_lifecycle_report,
    run_chat_message_with_context, run_chat_turn, run_chat_turn_with_events,
};
pub use types::{
    ChatContext, ChatMessageResult, ChatRunOptions, ChatTurnReport,
    DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET,
};
pub use workbench_diff::{
    WorkbenchDiffPreview, WorkbenchDiffStatus, collect_workbench_diff_status,
    workbench_diff_preview_text,
};
