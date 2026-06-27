// SPDX-License-Identifier: GPL-3.0-only
//! Context assembly primitives shared by runtime, session replay, and future UI/debug tools.

mod budget;
mod compressor;
mod diff;
mod error;
mod priority;
mod prompt;
mod references;
mod registry;
mod tokenizer;
mod types;

pub use budget::{
    HeuristicTokenEstimator, TokenEstimator, apply_context_token_budget, chat_context_token_count,
    estimate_tokens_heuristic,
};
pub use compressor::{ContextCompressionReport, TrajectoryCompressor};
pub use diff::diff_chat_context;
pub use error::{ContextError, ContextResult};
pub use priority::{
    ContextCompressedSection, ContextProtectionPolicy, ContextQuotaPolicy, PriorityContextEngine,
    PriorityContextReport,
};
pub use prompt::{
    PromptBuildMetadata, PromptBuildReport, PromptBuilder, PromptCachePlan, PromptCacheSectionPlan,
    PromptRedactionState, PromptSection, PromptSectionKind, PromptSectionMetadata,
    PromptSourceKind,
};
pub use references::{
    ensure_workspace_child, parse_context_references, resolve_context_reference,
    resolve_context_references,
};
pub use registry::{
    ContextEngineDescriptor, ContextEngineKind, ContextEngineRegistry, LlmSummaryCompressor,
    LlmSummaryRequest,
};
pub use tokenizer::{ContextTokenEstimator, ContextTokenizerKind};
pub use types::{
    ChatContext, ContextBudget, ContextBundle, ContextDiff, ContextDiffItem, ContextLimitReport,
    ContextReference, ContextReferenceKind, ContextSection, ContextSectionKind, ContextSourceKind,
    ContextTrustLevel, ResolvedContextReference,
};

pub const DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET: usize = 2_000;

#[cfg(test)]
mod registry_tests {
    use super::*;

    #[test]
    fn context_engine_registry_exposes_deterministic_and_llm_summary_engines() {
        let registry = ContextEngineRegistry;
        let engines = registry.descriptors();

        assert!(engines.iter().any(|engine| {
            engine.id == "deterministic"
                && engine.kind == ContextEngineKind::DeterministicCompressor
                && engine.default
        }));
        assert!(engines.iter().any(|engine| {
            engine.id == "llm-summary"
                && engine.kind == ContextEngineKind::LlmSummaryCompressor
                && !engine.default
                && engine.requires_model_provider
        }));
        assert_eq!(
            registry
                .default_descriptor()
                .expect("default context engine")
                .id,
            "deterministic"
        );
        assert_eq!(
            registry.supported_ids(),
            vec!["deterministic", "llm-summary"]
        );
    }

    #[test]
    fn llm_summary_compressor_prepares_redacted_summary_request() {
        let context = ChatContext {
            history: vec!["older turn with token=sk-secret-value".into()],
            retrieved_memory: vec!["project fact".into()],
            ..ChatContext::default()
        };

        let request = LlmSummaryCompressor
            .prepare_summary_request(&context, ContextBudget::new(128, "test-estimator"));

        assert_eq!(request.engine_id, "llm-summary");
        assert_eq!(request.max_summary_tokens, 128);
        assert!(request.system_prompt.contains("Summarize local context"));
        assert!(request.user_prompt.contains("older turn"));
        assert!(request.user_prompt.contains("project fact"));
        assert!(!request.user_prompt.contains("sk-secret-value"));
        assert!(request.user_prompt.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn llm_summary_compressor_builds_runtime_report_from_provider_summary() {
        let estimator = HeuristicTokenEstimator;
        let context = ChatContext {
            relationship: vec!["stable relationship fact".into()],
            references: vec!["explicit file reference".into()],
            history: vec![
                "older verbose turn detail one two three four five six seven".into(),
                "another verbose turn with token=sk-secret-value".into(),
            ],
            retrieved_memory: vec!["retrieved fact that may be summarized".into()],
            ..ChatContext::default()
        };
        let protected_tokens = estimator.estimate_tokens("stable relationship fact")
            + estimator.estimate_tokens("explicit file reference");
        let budget = ContextBudget::new(protected_tokens + 12, estimator.name());

        let report = LlmSummaryCompressor
            .compress_with_summary(
                context,
                budget,
                &estimator,
                "provider summary keeps the important facts token=sk-secret-value",
            )
            .expect("llm summary compression report");

        assert_eq!(
            report.context.relationship,
            vec!["stable relationship fact"]
        );
        assert_eq!(report.context.references, vec!["explicit file reference"]);
        assert!(
            report
                .context
                .history
                .iter()
                .any(|line| line.contains("[llm-summary] provider summary"))
        );
        assert!(
            !report
                .context
                .history
                .iter()
                .any(|line| line.contains("older verbose turn"))
        );
        assert!(
            !serde_json::to_string(&report)
                .expect("json")
                .contains("sk-secret-value")
        );
        assert!(report.after_tokens <= report.budget.max_tokens);
        assert!(report.summary.as_deref().is_some_and(|summary| {
            summary.contains("provider summary") && summary.contains("[REDACTED_SECRET]")
        }));
        assert!(
            report
                .continuation_prompt
                .as_deref()
                .is_some_and(|prompt| prompt.contains("LLM summary compressor"))
        );
    }
}
