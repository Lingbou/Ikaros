// SPDX-License-Identifier: GPL-3.0-only

use crate::{ChatContext, ContextSectionKind, PriorityContextEngine};

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
    PriorityContextEngine::default()
        .apply(context, budget, estimator)
        .context
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_budget_preserves_priority_and_truncates() {
        let estimator = HeuristicTokenEstimator;
        let context = ChatContext {
            relationship: vec!["rel".into()],
            references: vec!["ref".into()],
            history: vec!["hist".into()],
            memory: vec!["memory-context-is-long-enough-to-truncate".into()],
            rag: vec!["rag should be omitted".into()],
        };

        let budgeted = apply_context_token_budget(context, 12, &estimator);

        assert_eq!(budgeted.relationship, vec!["rel"]);
        assert_eq!(budgeted.references, vec!["ref"]);
        assert_eq!(budgeted.history, vec!["hist"]);
        assert!(budgeted.memory.is_empty());
        assert!(budgeted.rag.is_empty());
        assert!(chat_context_token_count(&budgeted, &estimator) <= 12);
    }
}
