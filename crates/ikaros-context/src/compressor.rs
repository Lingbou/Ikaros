// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    ChatContext, ContextBudget, ContextCompressedSection, ContextDiff, ContextLimitReport,
    ContextResult, PriorityContextEngine, TokenEstimator, chat_context_token_count,
    diff_chat_context,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCompressionReport {
    pub before_tokens: usize,
    pub after_tokens: usize,
    pub context: ChatContext,
    pub diff: ContextDiff,
    pub budget: ContextBudget,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compressed_sections: Vec<ContextCompressedSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_prompt: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TrajectoryCompressor {
    priority: PriorityContextEngine,
}

impl TrajectoryCompressor {
    pub fn new(priority: PriorityContextEngine) -> Self {
        Self { priority }
    }

    pub fn compress(
        &self,
        context: ChatContext,
        mut budget: ContextBudget,
        estimator: &dyn TokenEstimator,
    ) -> ContextResult<ContextCompressionReport> {
        let before_tokens = chat_context_token_count(&context, estimator);
        let protected_tokens = self.priority.protected_token_count(&context, estimator);
        if !budget.is_unbounded() && protected_tokens > budget.max_tokens {
            return Err(crate::ContextError::limit_exceeded(ContextLimitReport {
                max_tokens: budget.max_tokens,
                required_tokens: protected_tokens,
                protected_tokens,
                estimator: estimator.name().into(),
                protected_sections: self.priority.protected_sections(),
            }));
        }

        let priority_report = self
            .priority
            .apply(context.clone(), budget.max_tokens, estimator);
        let after_tokens = chat_context_token_count(&priority_report.context, estimator);
        if !budget.is_unbounded() && after_tokens > budget.max_tokens {
            return Err(crate::ContextError::limit_exceeded(ContextLimitReport {
                max_tokens: budget.max_tokens,
                required_tokens: after_tokens,
                protected_tokens,
                estimator: estimator.name().into(),
                protected_sections: self.priority.protected_sections(),
            }));
        }

        budget.used_tokens = after_tokens;
        let diff = diff_chat_context(&context, &priority_report.context, estimator);
        let summary = (!priority_report.compressed_sections.is_empty()).then(|| {
            priority_report
                .compressed_sections
                .iter()
                .map(|section| {
                    format!(
                        "{}: omitted {} line(s), about {} tokens",
                        section.section, section.omitted_lines, section.omitted_tokens
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        });
        let continuation_prompt = compression_continuation_prompt(
            &priority_report.compressed_sections,
            summary.as_deref(),
        );

        Ok(ContextCompressionReport {
            before_tokens,
            after_tokens,
            context: priority_report.context,
            diff,
            budget,
            compressed_sections: priority_report.compressed_sections,
            summary,
            continuation_prompt,
        })
    }
}

fn compression_continuation_prompt(
    compressed_sections: &[ContextCompressedSection],
    summary: Option<&str>,
) -> Option<String> {
    if compressed_sections.is_empty() {
        return None;
    }
    let section_list = compressed_sections
        .iter()
        .map(|section| section.section.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "Some local context was compacted before this turn to fit the model window. Compacted sections: {section_list}. Summary: {}. Treat visible local context as authoritative; do not invent omitted details, and ask for or use approved tools to recover specifics when needed.",
        summary.unwrap_or("context compacted")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ContextError, HeuristicTokenEstimator};

    #[test]
    fn compressor_preserves_protected_relationship_and_references() {
        let estimator = HeuristicTokenEstimator;
        let context = ChatContext {
            relationship: vec!["relationship fact".into()],
            references: vec!["explicit file reference".into()],
            history: vec!["older history detail that can be compacted".into()],
            memory: vec!["older memory detail that can be compacted".into()],
            rag: vec!["rag detail that can be compacted".into()],
        };
        let protected_tokens = estimator.estimate_tokens("relationship fact")
            + estimator.estimate_tokens("explicit file reference");
        let report = TrajectoryCompressor::default()
            .compress(
                context,
                ContextBudget::new(protected_tokens + 2, estimator.name()),
                &estimator,
            )
            .expect("compress");

        assert_eq!(report.context.relationship, vec!["relationship fact"]);
        assert_eq!(report.context.references, vec!["explicit file reference"]);
        assert!(report.after_tokens <= report.budget.max_tokens);
        assert!(report.compressed_sections.iter().all(|section| {
            !matches!(
                section.section,
                crate::ContextSectionKind::Relationship | crate::ContextSectionKind::References
            )
        }));
    }

    #[test]
    fn compressor_fails_when_protected_context_exceeds_budget() {
        let estimator = HeuristicTokenEstimator;
        let context = ChatContext {
            references: vec!["explicit reference must not be dropped".into()],
            ..ChatContext::default()
        };

        let error = TrajectoryCompressor::default()
            .compress(context, ContextBudget::new(1, estimator.name()), &estimator)
            .expect_err("protected context exceeds budget");

        match error {
            ContextError::LimitExceeded {
                max_tokens,
                protected_tokens,
                protected_sections,
                ..
            } => {
                assert_eq!(max_tokens, 1);
                assert!(protected_tokens > max_tokens);
                assert!(protected_sections.contains(&crate::ContextSectionKind::References));
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn compressor_emits_continuation_prompt_for_compacted_context() {
        let estimator = HeuristicTokenEstimator;
        let context = ChatContext {
            relationship: vec!["relationship fact".into()],
            history: vec!["older history detail that should be compacted".into()],
            memory: vec!["older memory detail that should be compacted".into()],
            ..ChatContext::default()
        };
        let protected_tokens = estimator.estimate_tokens("relationship fact");
        let report = TrajectoryCompressor::default()
            .compress(
                context,
                ContextBudget::new(protected_tokens + 2, estimator.name()),
                &estimator,
            )
            .expect("compress");

        let prompt = report.continuation_prompt.expect("continuation prompt");
        assert!(prompt.contains("Compacted sections"));
        assert!(prompt.contains("do not invent omitted details"));
    }
}
