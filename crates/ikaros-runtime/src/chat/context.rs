// SPDX-License-Identifier: GPL-3.0-only

use super::types::ChatContext;
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

pub fn apply_context_char_budget(context: ChatContext, budget: usize) -> ChatContext {
    if budget == 0 {
        return context;
    }
    let mut remaining = budget;
    ChatContext {
        relationship: budget_lines(context.relationship, &mut remaining),
        history: budget_lines(context.history, &mut remaining),
        memory: budget_lines(context.memory, &mut remaining),
        rag: budget_lines(context.rag, &mut remaining),
    }
}

pub fn chat_context_char_count(context: &ChatContext) -> usize {
    context
        .relationship
        .iter()
        .chain(context.history.iter())
        .chain(context.memory.iter())
        .chain(context.rag.iter())
        .map(|line| line.chars().count())
        .sum()
}

fn budget_lines(lines: Vec<String>, remaining: &mut usize) -> Vec<String> {
    let mut kept = Vec::new();
    for line in lines {
        if *remaining == 0 {
            break;
        }
        let line = redact_secrets(&line);
        let line_chars = line.chars().count();
        if line_chars <= *remaining {
            *remaining -= line_chars;
            kept.push(line);
            continue;
        }
        if let Some(truncated) = truncate_to_budget(&line, *remaining) {
            kept.push(truncated);
        }
        *remaining = 0;
        break;
    }
    kept
}

fn truncate_to_budget(line: &str, budget: usize) -> Option<String> {
    if budget == 0 {
        return None;
    }
    let suffix = "... [truncated]";
    let suffix_len = suffix.chars().count();
    if budget <= suffix_len {
        return Some(line.chars().take(budget).collect());
    }
    let mut output = line
        .chars()
        .take(budget.saturating_sub(suffix_len))
        .collect::<String>();
    output.push_str(suffix);
    Some(output)
}
