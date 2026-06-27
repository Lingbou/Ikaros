// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::context_engine::ContextBundle;
use ikaros_core::{Result, redact_secrets};
use ikaros_models::ModelResponse;
use ikaros_session::{
    PersistingAgentTurnSink, SessionEntry, SessionEntryId, SessionEntryKind, SessionId, TurnId,
};
use serde_json::json;

pub(super) struct ChatSessionEntryStats {
    pub(super) relationship_hits: usize,
    pub(super) reference_hits: usize,
    pub(super) memory_hits: usize,
    pub(super) rag_hits: usize,
}

pub(super) struct ChatAssistantEntryInput<'a> {
    pub(super) session_sink: Option<&'a PersistingAgentTurnSink>,
    pub(super) session_id: &'a SessionId,
    pub(super) turn_id: &'a TurnId,
    pub(super) user_entry_id: Option<SessionEntryId>,
    pub(super) agent: &'a str,
    pub(super) response: &'a ModelResponse,
    pub(super) streamed: bool,
    pub(super) stats: ChatSessionEntryStats,
}

pub(super) fn append_chat_user_session_entry(
    session_sink: Option<&PersistingAgentTurnSink>,
    session_id: &SessionId,
    turn_id: &TurnId,
    parent_entry_id: Option<SessionEntryId>,
    agent: &str,
    user_input: &str,
    content_block_count: usize,
) -> Result<Option<SessionEntryId>> {
    let Some(session_sink) = session_sink else {
        return Ok(None);
    };
    let redacted_user = redact_secrets(user_input);
    let mut user_entry = SessionEntry::new(session_id.clone(), SessionEntryKind::UserMessage);
    user_entry.parent_entry_id = parent_entry_id;
    user_entry.turn_id = Some(turn_id.clone());
    user_entry.visible_text = Some(redacted_user.clone());
    user_entry.payload = json!({
        "role": "user",
        "agent": redact_secrets(agent),
        "content": redacted_user,
        "content_block_count": content_block_count,
    });
    let entry_id = user_entry.entry_id.clone();
    session_sink.append_entry(&user_entry)?;
    Ok(Some(entry_id))
}

pub(super) fn append_context_compaction_session_entry(
    session_sink: Option<&PersistingAgentTurnSink>,
    session_id: &SessionId,
    turn_id: &TurnId,
    parent_entry_id: Option<SessionEntryId>,
    bundle: &ContextBundle,
) -> Result<Option<SessionEntryId>> {
    if bundle.compressed_sections.is_empty() {
        return Ok(None);
    }
    let Some(session_sink) = session_sink else {
        return Ok(None);
    };
    let Some(parent_entry_id) = parent_entry_id else {
        return Ok(None);
    };
    let summary = bundle
        .compression_summary
        .clone()
        .unwrap_or_else(|| "context compacted to fit model budget".into());
    let mut entry = SessionEntry::new(session_id.clone(), SessionEntryKind::Compaction);
    entry.parent_entry_id = Some(parent_entry_id);
    entry.turn_id = Some(turn_id.clone());
    entry.visible_text = Some(summary.clone());
    entry.payload = json!({
        "operation": "context_compaction",
        "summary": summary,
        "continuation_prompt": &bundle.continuation_prompt,
        "budget": &bundle.budget,
        "diff": &bundle.diff,
        "compressed_sections": &bundle.compressed_sections,
    });
    let entry_id = entry.entry_id.clone();
    session_sink.append_entry(&entry)?;
    Ok(Some(entry_id))
}

pub(super) fn append_chat_assistant_session_entry(
    input: ChatAssistantEntryInput<'_>,
) -> Result<()> {
    let Some(session_sink) = input.session_sink else {
        return Ok(());
    };
    let redacted_assistant = redact_secrets(&input.response.content);
    let mut assistant_entry =
        SessionEntry::new(input.session_id.clone(), SessionEntryKind::AssistantMessage);
    assistant_entry.parent_entry_id = input.user_entry_id;
    assistant_entry.turn_id = Some(input.turn_id.clone());
    assistant_entry.visible_text = Some(redacted_assistant.clone());
    assistant_entry.payload = json!({
        "role": "assistant",
        "agent": redact_secrets(input.agent),
        "provider": redact_secrets(&input.response.provider),
        "model": redact_secrets(&input.response.model),
        "streamed": input.streamed,
        "content": redacted_assistant,
        "relationship_hits": input.stats.relationship_hits,
        "reference_hits": input.stats.reference_hits,
        "memory_hits": input.stats.memory_hits,
        "rag_hits": input.stats.rag_hits,
        "usage": &input.response.usage,
    });
    session_sink.append_entry(&assistant_entry)
}
