// SPDX-License-Identifier: GPL-3.0-only

use thiserror::Error;

use crate::ContextLimitReport;

#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum ContextError {
    #[error("context reference path escapes workspace: {path}")]
    WorkspaceEscape { path: String },
    #[error("context reference path does not exist: {path}")]
    MissingPath { path: String },
    #[error("unsupported context reference: {reference}")]
    UnsupportedReference { reference: String },
    #[error("io error at {path}: {message}")]
    Io { path: String, message: String },
    #[error("git context reference failed for `{command}`: {stderr}")]
    Git { command: String, stderr: String },
    #[error(
        "context limit exceeded: required {required_tokens} token(s), budget allows {max_tokens} token(s), protected context uses {protected_tokens} token(s) with {estimator}"
    )]
    LimitExceeded {
        max_tokens: usize,
        required_tokens: usize,
        protected_tokens: usize,
        estimator: String,
        protected_sections: Vec<crate::ContextSectionKind>,
    },
}

pub type ContextResult<T> = std::result::Result<T, ContextError>;

impl ContextError {
    pub fn limit_exceeded(report: ContextLimitReport) -> Self {
        Self::LimitExceeded {
            max_tokens: report.max_tokens,
            required_tokens: report.required_tokens,
            protected_tokens: report.protected_tokens,
            estimator: report.estimator,
            protected_sections: report.protected_sections,
        }
    }
}
