// SPDX-License-Identifier: GPL-3.0-only

use crate::{ChatContext, ContextSectionKind};

pub trait TokenEstimator: Send + Sync {
    fn name(&self) -> &'static str;
    fn estimate_tokens(&self, text: &str) -> usize;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct HeuristicTokenEstimator;

impl TokenEstimator for HeuristicTokenEstimator {
    fn name(&self) -> &'static str {
        "heuristic-v1"
    }

    fn estimate_tokens(&self, text: &str) -> usize {
        estimate_tokens_heuristic(text)
    }
}

pub fn estimate_tokens_heuristic(text: &str) -> usize {
    let mut ascii_like = 0usize;
    let mut cjk_like = 0usize;
    for ch in text.chars() {
        if ch.is_whitespace() {
            continue;
        }
        if ('\u{4e00}'..='\u{9fff}').contains(&ch)
            || ('\u{3040}'..='\u{30ff}').contains(&ch)
            || ('\u{ac00}'..='\u{d7af}').contains(&ch)
        {
            cjk_like += 1;
        } else {
            ascii_like += 1;
        }
    }
    let ascii_tokens = ascii_like.div_ceil(4);
    let estimated = ascii_tokens + cjk_like;
    estimated.max(1)
}

pub fn chat_context_token_count(context: &ChatContext, estimator: &dyn TokenEstimator) -> usize {
    all_context_lines(context)
        .into_iter()
        .map(|(_, line)| estimator.estimate_tokens(line))
        .sum()
}

pub fn apply_context_token_budget(
    context: ChatContext,
    budget: usize,
    estimator: &dyn TokenEstimator,
) -> ChatContext {
    if budget == 0 {
        return context;
    }
    let mut remaining = budget;
    ChatContext {
        relationship: budget_lines(context.relationship, &mut remaining, estimator),
        references: budget_lines(context.references, &mut remaining, estimator),
        history: budget_lines(context.history, &mut remaining, estimator),
        memory: budget_lines(context.memory, &mut remaining, estimator),
        rag: budget_lines(context.rag, &mut remaining, estimator),
    }
}

pub(crate) fn all_context_lines(context: &ChatContext) -> Vec<(ContextSectionKind, &str)> {
    context
        .relationship
        .iter()
        .map(|line| (ContextSectionKind::Relationship, line.as_str()))
        .chain(
            context
                .references
                .iter()
                .map(|line| (ContextSectionKind::References, line.as_str())),
        )
        .chain(
            context
                .history
                .iter()
                .map(|line| (ContextSectionKind::History, line.as_str())),
        )
        .chain(
            context
                .memory
                .iter()
                .map(|line| (ContextSectionKind::Memory, line.as_str())),
        )
        .chain(
            context
                .rag
                .iter()
                .map(|line| (ContextSectionKind::Rag, line.as_str())),
        )
        .collect()
}

fn budget_lines(
    lines: Vec<String>,
    remaining: &mut usize,
    estimator: &dyn TokenEstimator,
) -> Vec<String> {
    let mut kept = Vec::new();
    for line in lines {
        if *remaining == 0 {
            break;
        }
        let line_tokens = estimator.estimate_tokens(&line);
        if line_tokens <= *remaining {
            *remaining -= line_tokens;
            kept.push(line);
            continue;
        }
        if let Some(truncated) = truncate_to_token_budget(&line, *remaining, estimator) {
            kept.push(truncated);
        }
        *remaining = 0;
        break;
    }
    kept
}

fn truncate_to_token_budget(
    line: &str,
    budget: usize,
    estimator: &dyn TokenEstimator,
) -> Option<String> {
    if budget == 0 {
        return None;
    }
    let suffix = "... [truncated]";
    if estimator.estimate_tokens(suffix) >= budget {
        return Some(line.chars().take(budget).collect());
    }
    let mut output = String::new();
    for ch in line.chars() {
        let candidate = format!("{output}{ch}{suffix}");
        if estimator.estimate_tokens(&candidate) > budget {
            break;
        }
        output.push(ch);
    }
    output.push_str(suffix);
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_budget_preserves_priority_and_truncates() {
        let estimator = HeuristicTokenEstimator;
        let context = ChatContext {
            relationship: vec!["relationship".into()],
            references: vec!["reference".into()],
            history: vec!["history".into()],
            memory: vec!["memory-context-is-long-enough-to-truncate".into()],
            rag: vec!["rag should be omitted".into()],
        };

        let budgeted = apply_context_token_budget(context, 14, &estimator);

        assert_eq!(budgeted.relationship, vec!["relationship"]);
        assert_eq!(budgeted.references, vec!["reference"]);
        assert_eq!(budgeted.history, vec!["history"]);
        assert_eq!(budgeted.memory.len(), 1);
        assert!(budgeted.memory[0].contains("[truncated]"));
        assert!(budgeted.rag.is_empty());
        assert!(chat_context_token_count(&budgeted, &estimator) <= 14);
    }
}
