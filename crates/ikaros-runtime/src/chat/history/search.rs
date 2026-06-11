// SPDX-License-Identifier: GPL-3.0-only

use super::ChatHistoryRecord;
use ikaros_core::redact_secrets;

pub(super) fn search_chat_history_records(
    records: Vec<ChatHistoryRecord>,
    query: &str,
    limit: usize,
    session_id: Option<&str>,
) -> Vec<ChatHistoryRecord> {
    let needle = redact_secrets(query).trim().to_lowercase();
    if needle.is_empty() || limit == 0 {
        return Vec::new();
    }
    records
        .into_iter()
        .rev()
        .filter(|record| {
            session_id.is_none_or(|session_id| record.session_id == session_id)
                && chat_history_record_matches(record, &needle)
        })
        .take(limit)
        .collect()
}

fn chat_history_record_matches(record: &ChatHistoryRecord, needle: &str) -> bool {
    [
        record.session_id.as_str(),
        record.turn_id.as_str(),
        record.created_at.as_str(),
        record.agent.as_str(),
        record.provider.as_str(),
        record.model.as_str(),
        record.user_message.as_str(),
        record.assistant_message.as_str(),
    ]
    .into_iter()
    .any(|field| redact_secrets(field).to_lowercase().contains(needle))
}
