// SPDX-License-Identifier: GPL-3.0-only

use super::ChatContext;
use ikaros_context::{
    ContextBudget, ContextCompressedSection, ContextCompressionReport, ContextDiff,
    ContextTokenizerKind,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactInput {
    pub context: ChatContext,
    pub budget: ContextBudget,
    pub tokenizer: ContextTokenizerKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactReport {
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

pub(super) fn compact_report_from_context_report(
    report: ContextCompressionReport,
) -> CompactReport {
    CompactReport {
        before_tokens: report.before_tokens,
        after_tokens: report.after_tokens,
        context: report.context,
        diff: report.diff,
        budget: report.budget,
        compressed_sections: report.compressed_sections,
        summary: report.summary,
        continuation_prompt: report.continuation_prompt,
    }
}
