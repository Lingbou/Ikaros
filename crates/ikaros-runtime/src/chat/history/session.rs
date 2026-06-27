// SPDX-License-Identifier: GPL-3.0-only

use super::{
    ChatHistoryRecord, ChatHistorySessionSummary, sessions::chat_history_session_summaries,
};
use ikaros_core::redact_secrets;
use ikaros_session::{SessionEntry, SessionEntryKind, SessionReplay};
use std::collections::HashMap;
use time::format_description::well_known::Rfc3339;

pub const CHAT_HISTORY_DELETE_SESSION_OPERATION: &str = "chat_history_delete_session";

pub fn chat_history_records_from_session_replay(replay: &SessionReplay) -> Vec<ChatHistoryRecord> {
    if session_replay_hides_chat_history(replay) {
        return Vec::new();
    }
    let entries_by_id: HashMap<_, _> = replay
        .entries
        .iter()
        .map(|entry| (entry.entry_id.as_str(), entry))
        .collect();
    let mut ordered_entries: Vec<_> = replay.entries.iter().collect();
    ordered_entries.sort_by(|left, right| {
        left.at
            .cmp(&right.at)
            .then_with(|| left.entry_id.as_str().cmp(right.entry_id.as_str()))
    });

    ordered_entries
        .iter()
        .filter(|entry| entry.kind == SessionEntryKind::AssistantMessage)
        .filter_map(|assistant| {
            let turn_id = assistant.turn_id.as_ref()?;
            let user = user_entry_for_assistant(assistant, &ordered_entries, &entries_by_id)?;
            let provider = non_empty_payload_string(assistant, "provider")?;
            let model = non_empty_payload_string(assistant, "model")?;
            Some(redacted_history_record(ChatHistoryRecord {
                session_id: replay.session.session_id.as_str().to_owned(),
                turn_id: turn_id.as_str().to_owned(),
                created_at: assistant
                    .at
                    .format(&Rfc3339)
                    .unwrap_or_else(|_| assistant.at.to_string()),
                agent: payload_string(assistant, "agent")
                    .or_else(|| payload_string(user, "agent"))
                    .or_else(|| replay.session.agent_id.clone())
                    .unwrap_or_default(),
                provider,
                model,
                streamed: assistant
                    .payload
                    .get("streamed")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false),
                user_message: entry_text(user),
                assistant_message: entry_text(assistant),
                relationship_hits: payload_usize(assistant, "relationship_hits"),
                memory_hits: payload_usize(assistant, "memory_hits"),
                rag_hits: payload_usize(assistant, "rag_hits"),
            }))
        })
        .collect()
}

pub fn chat_history_session_summaries_from_session_replays(
    replays: &[SessionReplay],
    limit: usize,
) -> Vec<ChatHistorySessionSummary> {
    let mut records = replays
        .iter()
        .flat_map(chat_history_records_from_session_replay)
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.session_id.cmp(&right.session_id))
            .then_with(|| left.turn_id.cmp(&right.turn_id))
    });
    chat_history_session_summaries(&records, limit)
}

pub fn session_replay_hides_chat_history(replay: &SessionReplay) -> bool {
    replay.entries.iter().any(|entry| {
        entry.kind == SessionEntryKind::Custom
            && entry
                .payload
                .get("operation")
                .and_then(serde_json::Value::as_str)
                == Some(CHAT_HISTORY_DELETE_SESSION_OPERATION)
    })
}

fn user_entry_for_assistant<'a>(
    assistant: &SessionEntry,
    ordered_entries: &'a [&SessionEntry],
    entries_by_id: &HashMap<&str, &'a SessionEntry>,
) -> Option<&'a SessionEntry> {
    let mut next_parent = assistant.parent_entry_id.as_ref();
    let mut remaining_hops = ordered_entries.len();
    while let Some(parent_id) = next_parent {
        if remaining_hops == 0 {
            break;
        }
        remaining_hops -= 1;
        let parent = entries_by_id.get(parent_id.as_str()).copied()?;
        if parent.kind == SessionEntryKind::UserMessage {
            return Some(parent);
        }
        next_parent = parent.parent_entry_id.as_ref();
    }

    let turn_id = assistant.turn_id.as_ref()?;
    ordered_entries.iter().rev().copied().find(|entry| {
        entry.kind == SessionEntryKind::UserMessage
            && entry.turn_id.as_ref() == Some(turn_id)
            && entry.at <= assistant.at
    })
}

fn entry_text(entry: &SessionEntry) -> String {
    entry
        .visible_text
        .clone()
        .or_else(|| payload_string(entry, "content"))
        .unwrap_or_default()
}

fn payload_string(entry: &SessionEntry, key: &str) -> Option<String> {
    entry
        .payload
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
}

fn non_empty_payload_string(entry: &SessionEntry, key: &str) -> Option<String> {
    payload_string(entry, key).filter(|value| !value.trim().is_empty())
}

fn payload_usize(entry: &SessionEntry, key: &str) -> usize {
    entry
        .payload
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or(0)
}

fn redacted_history_record(record: ChatHistoryRecord) -> ChatHistoryRecord {
    ChatHistoryRecord {
        session_id: redact_secrets(&record.session_id),
        turn_id: redact_secrets(&record.turn_id),
        created_at: redact_secrets(&record.created_at),
        agent: redact_secrets(&record.agent),
        provider: redact_secrets(&record.provider),
        model: redact_secrets(&record.model),
        streamed: record.streamed,
        user_message: redact_secrets(&record.user_message),
        assistant_message: redact_secrets(&record.assistant_message),
        relationship_hits: record.relationship_hits,
        memory_hits: record.memory_hits,
        rag_hits: record.rag_hits,
    }
}
