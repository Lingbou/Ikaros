// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    ChatContext, ContextBudget, ContextCompressedSection, ContextDiff, PriorityContextEngine,
    TokenEstimator, chat_context_token_count, diff_chat_context,
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
    ) -> ContextCompressionReport {
        let before_tokens = chat_context_token_count(&context, estimator);
        let priority_report = self
            .priority
            .apply(context.clone(), budget.max_tokens, estimator);
        let after_tokens = chat_context_token_count(&priority_report.context, estimator);
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

        ContextCompressionReport {
            before_tokens,
            after_tokens,
            context: priority_report.context,
            diff,
            budget,
            compressed_sections: priority_report.compressed_sections,
            summary,
        }
    }
}
