// SPDX-License-Identifier: GPL-3.0-only

use crate::{ChatContext, ContextSectionKind, TokenEstimator, chat_context_token_count};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextQuotaPolicy {
    pub relationship: u8,
    pub references: u8,
    pub history: u8,
    pub memory: u8,
    pub rag: u8,
}

impl Default for ContextQuotaPolicy {
    fn default() -> Self {
        Self {
            relationship: 10,
            references: 35,
            history: 20,
            memory: 20,
            rag: 15,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextCompressedSection {
    pub section: ContextSectionKind,
    pub omitted_lines: usize,
    pub omitted_tokens: usize,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PriorityContextReport {
    pub context: ChatContext,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compressed_sections: Vec<ContextCompressedSection>,
}

#[derive(Debug, Clone, Default)]
pub struct PriorityContextEngine {
    policy: ContextQuotaPolicy,
}

impl PriorityContextEngine {
    pub fn new(policy: ContextQuotaPolicy) -> Self {
        Self { policy }
    }

    pub fn apply(
        &self,
        context: ChatContext,
        budget: usize,
        estimator: &dyn TokenEstimator,
    ) -> PriorityContextReport {
        if budget == 0 || chat_context_token_count(&context, estimator) <= budget {
            return PriorityContextReport {
                context,
                compressed_sections: Vec::new(),
            };
        }

        let mut compressed_sections = Vec::new();
        let relationship = budget_section(
            ContextSectionKind::Relationship,
            context.relationship,
            quota_tokens(budget, self.policy.relationship),
            estimator,
            &mut compressed_sections,
        );
        let references = budget_section(
            ContextSectionKind::References,
            context.references,
            quota_tokens(budget, self.policy.references),
            estimator,
            &mut compressed_sections,
        );
        let history = budget_section(
            ContextSectionKind::History,
            context.history,
            quota_tokens(budget, self.policy.history),
            estimator,
            &mut compressed_sections,
        );
        let memory = budget_section(
            ContextSectionKind::Memory,
            context.memory,
            quota_tokens(budget, self.policy.memory),
            estimator,
            &mut compressed_sections,
        );
        let rag = budget_section(
            ContextSectionKind::Rag,
            context.rag,
            quota_tokens(budget, self.policy.rag),
            estimator,
            &mut compressed_sections,
        );

        let mut context = ChatContext {
            relationship,
            references,
            history,
            memory,
            rag,
        };
        enforce_total_budget(&mut context, budget, estimator);

        PriorityContextReport {
            context,
            compressed_sections,
        }
    }
}

fn quota_tokens(budget: usize, percent: u8) -> usize {
    let tokens = budget.saturating_mul(percent as usize) / 100;
    tokens.max(1)
}

fn budget_section(
    section: ContextSectionKind,
    lines: Vec<String>,
    budget: usize,
    estimator: &dyn TokenEstimator,
    compressed_sections: &mut Vec<ContextCompressedSection>,
) -> Vec<String> {
    if lines.is_empty() {
        return Vec::new();
    }

    let mut kept = Vec::new();
    let mut remaining = budget;
    let mut omitted = Vec::new();
    for line in lines {
        let tokens = estimator.estimate_tokens(&line);
        if tokens <= remaining {
            remaining -= tokens;
            kept.push(line);
        } else {
            omitted.push((line, tokens));
        }
    }

    if omitted.is_empty() {
        return kept;
    }

    let omitted_lines = omitted.len();
    let omitted_tokens = omitted.iter().map(|(_, tokens)| *tokens).sum::<usize>();
    let summary = section_summary(section, omitted_lines, omitted_tokens, &omitted[0].0);
    let summary_tokens = estimator.estimate_tokens(&summary);
    if summary_tokens <= remaining {
        kept.push(summary.clone());
    }
    compressed_sections.push(ContextCompressedSection {
        section,
        omitted_lines,
        omitted_tokens,
        summary,
    });
    kept
}

fn enforce_total_budget(context: &mut ChatContext, budget: usize, estimator: &dyn TokenEstimator) {
    if budget == 0 {
        return;
    }
    while chat_context_token_count(context, estimator) > budget {
        if context.rag.pop().is_some() {
            continue;
        }
        if context.memory.pop().is_some() {
            continue;
        }
        if context.history.pop().is_some() {
            continue;
        }
        if context.references.pop().is_some() {
            continue;
        }
        if context.relationship.pop().is_some() {
            continue;
        }
        break;
    }
}

fn section_summary(
    section: ContextSectionKind,
    omitted_lines: usize,
    omitted_tokens: usize,
    first_line: &str,
) -> String {
    format!(
        "[context summary: {section} omitted {omitted_lines} line(s), about {omitted_tokens} tokens; first: {}]",
        preview(first_line)
    )
}

fn preview(line: &str) -> String {
    const MAX_CHARS: usize = 80;
    let mut preview = line.chars().take(MAX_CHARS).collect::<String>();
    if line.chars().count() > MAX_CHARS {
        preview.push_str("...");
    }
    preview
}
