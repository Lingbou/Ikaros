// SPDX-License-Identifier: GPL-3.0-only

use super::{ChatHistoryRecord, ChatHistorySessionSummary, context::truncate_context_text};
use ikaros_core::redact_secrets;

pub(super) fn chat_history_session_summaries(
    records: &[ChatHistoryRecord],
    limit: usize,
) -> Vec<ChatHistorySessionSummary> {
    if limit == 0 {
        return Vec::new();
    }
    let mut summaries = Vec::<ChatHistorySessionSummary>::new();
    for record in records {
        if let Some(index) = summaries
            .iter()
            .position(|summary| summary.session_id == record.session_id)
        {
            let mut summary = summaries.remove(index);
            summary.turns += 1;
            summary.last_created_at = redact_secrets(&record.created_at);
            summary.last_turn_id = redact_secrets(&record.turn_id);
            summary.last_user_message = truncate_context_text(&record.user_message);
            summary.last_assistant_message = truncate_context_text(&record.assistant_message);
            push_unique_limited(&mut summary.agents, &record.agent, 3);
            push_unique_limited(&mut summary.providers, &record.provider, 3);
            push_unique_limited(&mut summary.models, &record.model, 3);
            summaries.push(summary);
            continue;
        }
        summaries.push(ChatHistorySessionSummary {
            session_id: redact_secrets(&record.session_id),
            turns: 1,
            first_created_at: redact_secrets(&record.created_at),
            last_created_at: redact_secrets(&record.created_at),
            last_turn_id: redact_secrets(&record.turn_id),
            agents: vec![redact_secrets(&record.agent)],
            providers: vec![redact_secrets(&record.provider)],
            models: vec![redact_secrets(&record.model)],
            last_user_message: truncate_context_text(&record.user_message),
            last_assistant_message: truncate_context_text(&record.assistant_message),
        });
    }
    summaries.reverse();
    summaries.truncate(limit);
    summaries
}

fn push_unique_limited(values: &mut Vec<String>, value: &str, limit: usize) {
    if values.len() >= limit {
        return;
    }
    let value = redact_secrets(value);
    if !values.contains(&value) {
        values.push(value);
    }
}
