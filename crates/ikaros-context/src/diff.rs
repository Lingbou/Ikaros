// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    ChatContext, ContextDiff, ContextDiffItem, ContextSectionKind, TokenEstimator,
    chat_context_token_count,
};

pub fn diff_chat_context(
    before: &ChatContext,
    after: &ChatContext,
    estimator: &dyn TokenEstimator,
) -> ContextDiff {
    let before_tokens = chat_context_token_count(before, estimator);
    let after_tokens = chat_context_token_count(after, estimator);
    let mut diff = ContextDiff {
        before_tokens,
        after_tokens,
        ..ContextDiff::default()
    };
    collect_section_diff(
        &mut diff,
        ContextSectionKind::Relationship,
        &before.relationship,
        &after.relationship,
        estimator,
    );
    collect_section_diff(
        &mut diff,
        ContextSectionKind::References,
        &before.references,
        &after.references,
        estimator,
    );
    collect_section_diff(
        &mut diff,
        ContextSectionKind::History,
        &before.history,
        &after.history,
        estimator,
    );
    collect_section_diff(
        &mut diff,
        ContextSectionKind::MemoryProjection,
        &before.memory_projection,
        &after.memory_projection,
        estimator,
    );
    collect_section_diff(
        &mut diff,
        ContextSectionKind::WorkingMemory,
        &before.working_memory,
        &after.working_memory,
        estimator,
    );
    collect_section_diff(
        &mut diff,
        ContextSectionKind::RetrievedMemory,
        &before.retrieved_memory,
        &after.retrieved_memory,
        estimator,
    );
    collect_section_diff(
        &mut diff,
        ContextSectionKind::Rag,
        &before.rag,
        &after.rag,
        estimator,
    );
    diff
}

fn collect_section_diff(
    diff: &mut ContextDiff,
    section: ContextSectionKind,
    before: &[String],
    after: &[String],
    estimator: &dyn TokenEstimator,
) {
    for line in after {
        if !before.contains(line) {
            diff.added.push(diff_item(section, line, estimator));
        }
    }
    for line in before {
        if !after.contains(line) {
            if after.iter().any(|kept| is_context_summary_line(kept)) {
                diff.compressed.push(diff_item(section, line, estimator));
            } else {
                diff.removed.push(diff_item(section, line, estimator));
            }
        }
    }
}

fn is_context_summary_line(line: &str) -> bool {
    line.trim_start().starts_with("[context summary:")
}

fn diff_item(
    section: ContextSectionKind,
    line: &str,
    estimator: &dyn TokenEstimator,
) -> ContextDiffItem {
    ContextDiffItem {
        section,
        tokens: estimator.estimate_tokens(line),
        preview: preview(line),
    }
}

fn preview(line: &str) -> String {
    const MAX_CHARS: usize = 120;
    let mut preview = line.chars().take(MAX_CHARS).collect::<String>();
    if line.chars().count() > MAX_CHARS {
        preview.push_str("...");
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatContext, HeuristicTokenEstimator};

    #[test]
    fn context_summary_marks_removed_lines_as_compressed() {
        let before = ChatContext {
            retrieved_memory: vec!["older memory detail".into()],
            ..ChatContext::default()
        };
        let after = ChatContext {
            retrieved_memory: vec!["[context summary: retrieved_memory omitted 1 line(s)]".into()],
            ..ChatContext::default()
        };

        let diff = diff_chat_context(&before, &after, &HeuristicTokenEstimator);

        assert_eq!(diff.compressed.len(), 1);
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn reference_truncation_marker_does_not_imply_context_compression() {
        let before = ChatContext {
            references: vec!["full reference detail".into()],
            ..ChatContext::default()
        };
        let after = ChatContext {
            references: vec!["reference preview\n... [truncated]".into()],
            ..ChatContext::default()
        };

        let diff = diff_chat_context(&before, &after, &HeuristicTokenEstimator);

        assert!(diff.compressed.is_empty());
        assert_eq!(diff.removed.len(), 1);
    }
}
