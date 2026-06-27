// SPDX-License-Identifier: GPL-3.0-only

use super::ChatHistoryRecord;
use ikaros_core::redact_secrets;
use uuid::Uuid;

pub fn new_chat_session_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn chat_history_context_lines(records: &[ChatHistoryRecord], limit: usize) -> Vec<String> {
    if limit == 0 {
        return Vec::new();
    }
    let start = records.len().saturating_sub(limit);
    records[start..]
        .iter()
        .map(|record| {
            redact_secrets(&format!(
                "[{} agent={} provider={} model={}] user: {} | assistant: {}",
                record.created_at,
                record.agent,
                record.provider,
                record.model,
                truncate_context_text(&record.user_message),
                truncate_context_text(&record.assistant_message)
            ))
        })
        .collect()
}

pub fn chat_history_context_lines_with_summary(
    records: &[ChatHistoryRecord],
    recent_limit: usize,
    summary_limit: usize,
) -> Vec<String> {
    if records.is_empty() || (recent_limit == 0 && summary_limit == 0) {
        return Vec::new();
    }
    let recent_start = records.len().saturating_sub(recent_limit);
    let mut lines = Vec::new();
    if summary_limit > 0 && recent_start > 0 {
        let summary_start = recent_start.saturating_sub(summary_limit);
        if let Some(summary) = chat_history_summary_line(&records[summary_start..recent_start]) {
            lines.push(summary);
        }
    }
    if recent_limit > 0 {
        lines.extend(chat_history_context_lines(
            &records[recent_start..],
            recent_limit,
        ));
    }
    lines
}

fn chat_history_summary_line(records: &[ChatHistoryRecord]) -> Option<String> {
    let first = records.first()?;
    let last = records.last()?;
    let agents = unique_join(records.iter().map(|record| record.agent.as_str()), 3);
    let providers = unique_join(records.iter().map(|record| record.provider.as_str()), 3);
    let user_summary = summarize_history_texts(
        records.iter().map(|record| record.user_message.as_str()),
        360,
    );
    let assistant_summary = summarize_history_texts(
        records
            .iter()
            .map(|record| record.assistant_message.as_str()),
        240,
    );
    Some(redact_secrets(&format!(
        "[older chat summary turns={} first={} last={} agents={} providers={}] user: {} | assistant: {}",
        records.len(),
        first.created_at,
        last.created_at,
        agents,
        providers,
        user_summary,
        assistant_summary
    )))
}

fn unique_join<'a>(values: impl Iterator<Item = &'a str>, limit: usize) -> String {
    let mut unique = Vec::<String>::new();
    for value in values {
        let value = redact_secrets(value);
        if !unique.contains(&value) {
            unique.push(value);
        }
        if unique.len() >= limit {
            break;
        }
    }
    if unique.is_empty() {
        "none".into()
    } else {
        unique.join(",")
    }
}

fn summarize_history_texts<'a>(values: impl Iterator<Item = &'a str>, max_chars: usize) -> String {
    let mut output = String::new();
    for value in values {
        let value = truncate_context_text(&redact_secrets(value));
        if value.trim().is_empty() {
            continue;
        }
        if !output.is_empty() {
            output.push_str(" / ");
        }
        output.push_str(&value);
        if output.chars().count() >= max_chars {
            return truncate_chars(&output, max_chars);
        }
    }
    if output.is_empty() {
        "none".into()
    } else {
        truncate_chars(&output, max_chars)
    }
}

pub(super) fn truncate_context_text(text: &str) -> String {
    const MAX_CHARS: usize = 280;
    truncate_chars(text, MAX_CHARS)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index >= max_chars {
            output.push_str("...");
            return output;
        }
        output.push(ch);
    }
    output
}
