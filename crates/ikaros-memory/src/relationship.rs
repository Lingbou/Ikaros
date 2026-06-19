// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::redact_secrets;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RelationshipMemoryNote {
    pub id: String,
    pub scope: String,
    pub content: String,
    pub tags: Vec<String>,
    pub created_at: String,
    pub updated_at: Option<String>,
}

pub fn relationship_notes_from_output(
    output: &serde_json::Value,
    limit: usize,
) -> Vec<RelationshipMemoryNote> {
    output
        .as_array()
        .into_iter()
        .flatten()
        .filter(|record| {
            record
                .get("kind")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind.eq_ignore_ascii_case("relationship"))
        })
        .take(limit)
        .filter_map(|record| {
            let id = record.get("id").and_then(serde_json::Value::as_str)?;
            let scope = record.get("scope").and_then(serde_json::Value::as_str)?;
            let content = record.get("content").and_then(serde_json::Value::as_str)?;
            let created_at = record
                .get("created_at")
                .and_then(serde_json::Value::as_str)?;
            let updated_at = record
                .get("updated_at")
                .and_then(serde_json::Value::as_str)
                .map(redact_secrets);
            let tags = record
                .get("tags")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(redact_secrets)
                .collect();
            Some(RelationshipMemoryNote {
                id: redact_secrets(id),
                scope: redact_secrets(scope),
                content: redact_secrets(content),
                tags,
                created_at: redact_secrets(created_at),
                updated_at,
            })
        })
        .collect()
}

pub fn relationship_context_lines(notes: &[RelationshipMemoryNote], limit: usize) -> Vec<String> {
    notes
        .iter()
        .take(limit)
        .map(|note| {
            let tags = if note.tags.is_empty() {
                String::new()
            } else {
                format!(" tags={}", note.tags.join(","))
            };
            redact_secrets(&format!(
                "[relationship/{}] {}{}",
                note.scope, note.content, tags
            ))
        })
        .collect()
}
