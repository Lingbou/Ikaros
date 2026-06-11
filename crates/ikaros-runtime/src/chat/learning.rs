// SPDX-License-Identifier: GPL-3.0-only

use super::types::ChatRunOptions;
use ikaros_core::{Result, contains_secret_like, redact_secrets};
use ikaros_harness::{AuditEvent, ExecutionSession, SkillRegistry};
use serde_json::json;

const MAX_CANDIDATES_PER_TURN: usize = 3;
const MAX_MEMORY_CHARS: usize = 180;

pub async fn learn_relationships_from_chat(
    input: &str,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<usize> {
    if !options.relationship_learning {
        return Ok(0);
    }
    let scope = options.scope.as_deref().unwrap_or("default");
    let mut learned = 0;
    for candidate in extract_relationship_memory_candidates(input) {
        if relationship_memory_exists(session, registry, scope, &candidate).await? {
            continue;
        }
        let result = session
            .execute_skill(
                registry,
                "memory_append",
                json!({
                    "kind": "relationship",
                    "scope": scope,
                    "content": candidate,
                    "tags": ["relationship", "chat-learned"],
                }),
            )
            .await?;
        if result.ok {
            learned += 1;
        }
    }
    if learned > 0 {
        session.audit.append(AuditEvent::new(
            "chat_relationship_learned",
            None,
            "relationship memory learned from chat",
            json!({
                "learned": learned,
                "scope": redact_secrets(scope),
            }),
        )?)?;
    }
    Ok(learned)
}

async fn relationship_memory_exists(
    session: &ExecutionSession,
    registry: &SkillRegistry,
    scope: &str,
    candidate: &str,
) -> Result<bool> {
    let result = session
        .execute_read_skill_with_audit_input(
            registry,
            "memory_search",
            json!({
                "kind": "relationship",
                "scope": scope,
                "query": candidate,
                "limit": 5,
            }),
            json!({
                "kind": "relationship",
                "scope": redact_secrets(scope),
                "query": "<redacted learned relationship>",
                "limit": 5,
            }),
        )
        .await?;
    Ok(result
        .output
        .as_array()
        .into_iter()
        .flatten()
        .any(|record| {
            record
                .get("content")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|content| content == candidate)
        }))
}

pub fn extract_relationship_memory_candidates(input: &str) -> Vec<String> {
    let redacted = redact_secrets(input);
    split_statements(&redacted)
        .into_iter()
        .filter_map(|statement| relationship_candidate_from_statement(&statement))
        .filter(|candidate| !contains_secret_like(candidate))
        .fold(Vec::<String>::new(), |mut candidates, candidate| {
            if candidates.len() < MAX_CANDIDATES_PER_TURN && !candidates.contains(&candidate) {
                candidates.push(candidate);
            }
            candidates
        })
}

fn relationship_candidate_from_statement(statement: &str) -> Option<String> {
    let statement = clean_fragment(statement);
    if statement.is_empty() {
        return None;
    }

    if let Some(value) = candidate_after_any(
        &statement,
        &[
            "call me ",
            "please call me ",
            "my name is ",
            "以后叫我",
            "请叫我",
            "叫我",
            "我的名字是",
        ],
    ) {
        return normalized_candidate("User preferred name", value);
    }

    if let Some(value) = candidate_after_any(
        &statement,
        &[
            "i prefer ",
            "i like ",
            "i love ",
            "i hate ",
            "i don't like ",
            "i do not like ",
            "我偏好",
            "我更喜欢",
            "我喜欢",
            "我不喜欢",
            "我讨厌",
        ],
    ) {
        return normalized_candidate("User preference", value);
    }

    if let Some(value) = candidate_after_any(
        &statement,
        &[
            "remember that ",
            "please remember that ",
            "please remember ",
            "记住",
            "请记住",
        ],
    ) {
        return normalized_candidate("User asked Ikaros to remember", value);
    }

    if let Some(value) = candidate_after_any(
        &statement,
        &[
            "i want you to ",
            "i need you to ",
            "我希望你",
            "我想让你",
            "我需要你",
        ],
    ) {
        return normalized_candidate("User expectation", value);
    }

    None
}

fn candidate_after_any<'a>(statement: &'a str, patterns: &[&str]) -> Option<&'a str> {
    let lower = statement.to_ascii_lowercase();
    patterns.iter().find_map(|pattern| {
        let index = lower.find(pattern)?;
        Some(&statement[index + pattern.len()..])
    })
}

fn normalized_candidate(prefix: &str, value: &str) -> Option<String> {
    let value = clean_fragment(value);
    if value.chars().count() < 2 {
        return None;
    }
    Some(format!(
        "{prefix}: {}",
        truncate_chars(&value, MAX_MEMORY_CHARS)
    ))
}

fn split_statements(input: &str) -> Vec<String> {
    input
        .split(['\n', '.', '!', '?', ';', '。', '！', '？', '；'])
        .map(clean_fragment)
        .filter(|statement| !statement.is_empty())
        .collect()
}

fn clean_fragment(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("that ")
        .trim_start_matches("that:")
        .trim_matches(|ch: char| {
            ch.is_whitespace()
                || matches!(
                    ch,
                    '"' | '\'' | '`' | ':' | ',' | '，' | '：' | '。' | '！' | '？'
                )
        })
        .trim()
        .to_owned()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut output = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            output.push_str("...");
            return output;
        }
        output.push(ch);
    }
    output
}
