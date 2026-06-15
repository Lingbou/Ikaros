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
pub struct ContextProtectionPolicy {
    pub relationship: bool,
    pub references: bool,
}

impl Default for ContextProtectionPolicy {
    fn default() -> Self {
        Self {
            relationship: true,
            references: true,
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
    protection: ContextProtectionPolicy,
}

impl PriorityContextEngine {
    pub fn new(policy: ContextQuotaPolicy) -> Self {
        Self {
            policy,
            protection: ContextProtectionPolicy::default(),
        }
    }

    pub fn with_protection(mut self, protection: ContextProtectionPolicy) -> Self {
        self.protection = protection;
        self
    }

    pub fn protected_sections(&self) -> Vec<ContextSectionKind> {
        let mut sections = Vec::new();
        if self.protection.relationship {
            sections.push(ContextSectionKind::Relationship);
        }
        if self.protection.references {
            sections.push(ContextSectionKind::References);
        }
        sections
    }

    pub fn protected_token_count(
        &self,
        context: &ChatContext,
        estimator: &dyn TokenEstimator,
    ) -> usize {
        let mut tokens = 0usize;
        if self.protection.relationship {
            tokens += section_token_count(&context.relationship, estimator);
        }
        if self.protection.references {
            tokens += section_token_count(&context.references, estimator);
        }
        tokens
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

        let protected_tokens = self.protected_token_count(&context, estimator);
        let available = budget.saturating_sub(protected_tokens);
        let allocations = ContextBudgetAllocation::new(available, &self.policy, &self.protection);

        let mut compressed_sections = Vec::new();
        let relationship = if self.protection.relationship {
            context.relationship
        } else {
            budget_section(
                ContextSectionKind::Relationship,
                context.relationship,
                allocations.relationship,
                estimator,
                &mut compressed_sections,
            )
        };
        let references = if self.protection.references {
            context.references
        } else {
            budget_section(
                ContextSectionKind::References,
                context.references,
                allocations.references,
                estimator,
                &mut compressed_sections,
            )
        };
        let history = budget_section(
            ContextSectionKind::History,
            context.history,
            allocations.history,
            estimator,
            &mut compressed_sections,
        );
        let memory = budget_section(
            ContextSectionKind::Memory,
            context.memory,
            allocations.memory,
            estimator,
            &mut compressed_sections,
        );
        let rag = budget_section(
            ContextSectionKind::Rag,
            context.rag,
            allocations.rag,
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
        enforce_total_budget(&mut context, budget, estimator, &self.protection);

        PriorityContextReport {
            context,
            compressed_sections,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ContextBudgetAllocation {
    relationship: usize,
    references: usize,
    history: usize,
    memory: usize,
    rag: usize,
}

impl ContextBudgetAllocation {
    fn new(
        available: usize,
        policy: &ContextQuotaPolicy,
        protection: &ContextProtectionPolicy,
    ) -> Self {
        let weights = [
            (!protection.relationship).then_some(policy.relationship),
            (!protection.references).then_some(policy.references),
            Some(policy.history),
            Some(policy.memory),
            Some(policy.rag),
        ];
        let total_weight = weights
            .into_iter()
            .flatten()
            .map(usize::from)
            .sum::<usize>();
        if available == 0 || total_weight == 0 {
            return Self::default();
        }

        let mut remaining_budget = available;
        let mut remaining_weight = total_weight;
        let mut next = |weight: Option<u8>| {
            let Some(weight) = weight else {
                return 0;
            };
            if remaining_weight == 0 {
                return 0;
            }
            let weight = usize::from(weight);
            let tokens = remaining_budget.saturating_mul(weight) / remaining_weight;
            remaining_budget = remaining_budget.saturating_sub(tokens);
            remaining_weight = remaining_weight.saturating_sub(weight);
            tokens
        };

        Self {
            relationship: next((!protection.relationship).then_some(policy.relationship)),
            references: next((!protection.references).then_some(policy.references)),
            history: next(Some(policy.history)),
            memory: next(Some(policy.memory)),
            rag: next(Some(policy.rag)),
        }
    }
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

fn enforce_total_budget(
    context: &mut ChatContext,
    budget: usize,
    estimator: &dyn TokenEstimator,
    protection: &ContextProtectionPolicy,
) {
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
        if !protection.references && context.references.pop().is_some() {
            continue;
        }
        if !protection.relationship && context.relationship.pop().is_some() {
            continue;
        }
        break;
    }
}

fn section_token_count(lines: &[String], estimator: &dyn TokenEstimator) -> usize {
    lines
        .iter()
        .map(|line| estimator.estimate_tokens(line))
        .sum()
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
