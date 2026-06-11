// SPDX-License-Identifier: GPL-3.0-only

use super::types::AgentLoopToolCallParseStrategy;

#[derive(Debug, Clone)]
pub(super) struct JsonCandidate {
    pub(super) strategy: AgentLoopToolCallParseStrategy,
    pub(super) json: String,
}

pub(super) fn extract_json_candidates(content: &str) -> Vec<JsonCandidate> {
    let mut candidates = Vec::new();
    if let Some(fenced) = extract_fenced_json(content) {
        candidates.push(JsonCandidate {
            strategy: AgentLoopToolCallParseStrategy::FencedJson,
            json: fenced,
        });
    }
    if let Some(object) = extract_between(content, '{', '}') {
        candidates.push(JsonCandidate {
            strategy: AgentLoopToolCallParseStrategy::EmbeddedJsonObject,
            json: object,
        });
    }
    if let Some(array) = extract_between(content, '[', ']') {
        candidates.push(JsonCandidate {
            strategy: AgentLoopToolCallParseStrategy::EmbeddedJsonArray,
            json: array,
        });
    }
    candidates
}

fn extract_fenced_json(content: &str) -> Option<String> {
    let fence_start = content.find("```")?;
    let after_start = &content[fence_start + 3..];
    let content_start = after_start
        .strip_prefix("json")
        .or_else(|| after_start.strip_prefix("JSON"))
        .unwrap_or(after_start)
        .trim_start_matches(['\r', '\n']);
    let fence_end = content_start.find("```")?;
    Some(content_start[..fence_end].trim().to_string())
}

fn extract_between(content: &str, start: char, end: char) -> Option<String> {
    let start_index = content.find(start)?;
    let end_index = content.rfind(end)?;
    (end_index > start_index).then(|| content[start_index..=end_index].trim().to_string())
}
