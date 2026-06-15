// SPDX-License-Identifier: GPL-3.0-only

use ikaros_context::ChatContext;
use ikaros_core::{RiskLevel, redact_secrets};
use ikaros_harness::SkillRegistry;

pub fn context_lookup_is_safe_read(registry: &SkillRegistry, name: &str) -> bool {
    registry
        .get(name)
        .is_some_and(|skill| skill.risk_level() == RiskLevel::SafeRead)
}

pub fn extract_memory_context(output: &serde_json::Value, limit: usize) -> Vec<String> {
    output
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|record| {
            let kind = record.get("kind").and_then(serde_json::Value::as_str)?;
            if kind.eq_ignore_ascii_case("relationship") {
                return None;
            }
            let scope = record
                .get("scope")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let content = record.get("content").and_then(serde_json::Value::as_str)?;
            Some(redact_secrets(&format!("[{kind}/{scope}] {content}")))
        })
        .take(limit)
        .collect()
}

pub fn extract_rag_context(output: &serde_json::Value, limit: usize) -> Vec<String> {
    output
        .as_array()
        .into_iter()
        .flatten()
        .take(limit)
        .filter_map(|hit| {
            let chunk = hit.get("chunk")?;
            let content = chunk.get("content").and_then(serde_json::Value::as_str)?;
            let citation = hit.get("citation")?;
            let path = citation
                .get("source_path")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("unknown");
            let line_start = citation
                .get("line_start")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            let line_end = citation
                .get("line_end")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            Some(redact_secrets(&format!(
                "[{path}:{line_start}-{line_end}] {content}"
            )))
        })
        .collect()
}

pub fn redact_chat_context(context: ChatContext) -> ChatContext {
    ChatContext {
        relationship: redact_lines(context.relationship),
        references: redact_lines(context.references),
        history: redact_lines(context.history),
        memory: redact_lines(context.memory),
        rag: redact_lines(context.rag),
    }
}

fn redact_lines(lines: Vec<String>) -> Vec<String> {
    lines
        .into_iter()
        .map(|line| redact_secrets(&line))
        .collect()
}
