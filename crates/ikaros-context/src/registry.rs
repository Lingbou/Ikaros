// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    ChatContext, ContextBudget, ContextCompressionReport, ContextResult, TokenEstimator,
    TrajectoryCompressor, chat_context_token_count, diff_chat_context,
};
use ikaros_core::redact_secrets;
use serde::{Deserialize, Serialize};

const LLM_SUMMARY_MAX_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextEngineKind {
    DeterministicCompressor,
    LlmSummaryCompressor,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextEngineDescriptor {
    pub id: &'static str,
    pub kind: ContextEngineKind,
    pub summary: &'static str,
    pub default: bool,
    pub requires_model_provider: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ContextEngineRegistry;

impl ContextEngineRegistry {
    pub fn descriptors(&self) -> Vec<ContextEngineDescriptor> {
        vec![
            ContextEngineDescriptor {
                id: "deterministic",
                kind: ContextEngineKind::DeterministicCompressor,
                summary: "local priority compressor with protected context boundaries",
                default: true,
                requires_model_provider: false,
            },
            ContextEngineDescriptor {
                id: "llm-summary",
                kind: ContextEngineKind::LlmSummaryCompressor,
                summary: "provider-backed summary compressor request builder",
                default: false,
                requires_model_provider: true,
            },
        ]
    }

    pub fn descriptor(&self, id: &str) -> Option<ContextEngineDescriptor> {
        self.descriptors()
            .into_iter()
            .find(|descriptor| descriptor.id == id)
    }

    pub fn default_descriptor(&self) -> Option<ContextEngineDescriptor> {
        self.descriptors()
            .into_iter()
            .find(|descriptor| descriptor.default)
    }

    pub fn supported_ids(&self) -> Vec<&'static str> {
        self.descriptors()
            .into_iter()
            .map(|descriptor| descriptor.id)
            .collect()
    }
}

#[derive(Debug, Clone, Default)]
pub struct LlmSummaryCompressor;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LlmSummaryRequest {
    pub engine_id: &'static str,
    pub system_prompt: String,
    pub user_prompt: String,
    pub max_summary_tokens: usize,
}

impl LlmSummaryCompressor {
    pub fn prepare_summary_request(
        &self,
        context: &ChatContext,
        budget: ContextBudget,
    ) -> LlmSummaryRequest {
        LlmSummaryRequest {
            engine_id: "llm-summary",
            system_prompt: "Summarize local context for the next Ikaros turn. Preserve explicit user instructions, file/reference facts, tool results, and unresolved decisions. Do not invent omitted details. Redact secrets.".into(),
            user_prompt: redact_secrets(&context_lines(context).join("\n")),
            max_summary_tokens: budget.max_tokens.max(1),
        }
    }

    pub fn compress_with_summary(
        &self,
        context: ChatContext,
        budget: ContextBudget,
        estimator: &dyn TokenEstimator,
        provider_summary: impl AsRef<str>,
    ) -> ContextResult<ContextCompressionReport> {
        let context = redact_context(context);
        let deterministic =
            TrajectoryCompressor::default().compress(context.clone(), budget.clone(), estimator)?;
        if deterministic.compressed_sections.is_empty() {
            return Ok(deterministic);
        }

        let summary = sanitize_summary(provider_summary.as_ref());
        let mut summarized_context = deterministic.context.clone();
        if let Some(summary_line) =
            summary_line_for_budget(&summarized_context, &summary, &budget, estimator)
        {
            summarized_context.history.insert(0, summary_line);
        }

        let after_tokens = chat_context_token_count(&summarized_context, estimator);
        let mut budget = deterministic.budget;
        budget.used_tokens = after_tokens;
        let diff = diff_chat_context(&context, &summarized_context, estimator);

        Ok(ContextCompressionReport {
            before_tokens: deterministic.before_tokens,
            after_tokens,
            context: summarized_context,
            diff,
            budget,
            compressed_sections: deterministic.compressed_sections,
            summary: Some(summary.clone()),
            continuation_prompt: Some(llm_summary_continuation_prompt(&summary)),
        })
    }
}

fn sanitize_summary(summary: &str) -> String {
    let redacted = redact_secrets(summary);
    let normalized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= LLM_SUMMARY_MAX_CHARS {
        return normalized;
    }
    let mut capped = normalized
        .chars()
        .take(LLM_SUMMARY_MAX_CHARS)
        .collect::<String>();
    capped.push_str("...[truncated]");
    capped
}

fn summary_line_for_budget(
    context: &ChatContext,
    summary: &str,
    budget: &ContextBudget,
    estimator: &dyn TokenEstimator,
) -> Option<String> {
    if summary.trim().is_empty() {
        return None;
    }
    if budget.is_unbounded() {
        return Some(format!("[llm-summary] {summary}"));
    }

    let context_tokens = chat_context_token_count(context, estimator);
    let available_tokens = budget.max_tokens.saturating_sub(context_tokens);
    fit_summary_line_to_tokens(summary, available_tokens, estimator)
}

fn fit_summary_line_to_tokens(
    summary: &str,
    max_tokens: usize,
    estimator: &dyn TokenEstimator,
) -> Option<String> {
    let full_line = format!("[llm-summary] {summary}");
    if estimator.estimate_tokens(&full_line) <= max_tokens {
        return Some(full_line);
    }

    let mut fitted = String::new();
    for word in summary.split_whitespace() {
        let candidate = if fitted.is_empty() {
            word.to_owned()
        } else {
            format!("{fitted} {word}")
        };
        let candidate_line = format!("[llm-summary] {candidate}...[truncated]");
        if estimator.estimate_tokens(&candidate_line) > max_tokens {
            break;
        }
        fitted = candidate;
    }

    (!fitted.is_empty()).then(|| format!("[llm-summary] {fitted}...[truncated]"))
}

fn llm_summary_continuation_prompt(summary: &str) -> String {
    format!(
        "Local context was compacted by the LLM summary compressor before this turn. Summary: {summary}. Treat visible local context and explicit references as authoritative; do not invent omitted details."
    )
}

fn context_lines(context: &ChatContext) -> Vec<String> {
    let mut lines = Vec::new();
    append_section(&mut lines, "relationship", &context.relationship);
    append_section(&mut lines, "references", &context.references);
    append_section(&mut lines, "history", &context.history);
    append_section(&mut lines, "memory_projection", &context.memory_projection);
    append_section(&mut lines, "working_memory", &context.working_memory);
    append_section(&mut lines, "retrieved_memory", &context.retrieved_memory);
    append_section(&mut lines, "rag", &context.rag);
    lines
}

fn append_section(lines: &mut Vec<String>, label: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    lines.push(format!("## {label}"));
    lines.extend(values.iter().map(|value| format!("- {value}")));
}

fn redact_context(mut context: ChatContext) -> ChatContext {
    redact_lines(&mut context.relationship);
    redact_lines(&mut context.references);
    redact_lines(&mut context.history);
    redact_lines(&mut context.memory_projection);
    redact_lines(&mut context.working_memory);
    redact_lines(&mut context.retrieved_memory);
    redact_lines(&mut context.rag);
    context
}

fn redact_lines(lines: &mut [String]) {
    for line in lines {
        *line = redact_secrets(line);
    }
}
