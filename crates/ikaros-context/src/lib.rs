// SPDX-License-Identifier: GPL-3.0-only
//! Context assembly primitives shared by runtime, session replay, and future UI/debug tools.

mod budget;
mod compressor;
mod diff;
mod error;
mod priority;
mod references;
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
pub use references::{
    ensure_workspace_child, parse_context_references, resolve_context_reference,
    resolve_context_references,
};
pub use tokenizer::{ContextTokenEstimator, ContextTokenizerKind};
pub use types::{
    ChatContext, ContextBudget, ContextBundle, ContextDiff, ContextDiffItem, ContextLimitReport,
    ContextReference, ContextReferenceKind, ContextSection, ContextSectionKind,
    ResolvedContextReference,
};

pub const DEFAULT_CHAT_CONTEXT_TOKEN_BUDGET: usize = 2_000;
