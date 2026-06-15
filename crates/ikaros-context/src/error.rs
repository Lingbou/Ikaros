// SPDX-License-Identifier: GPL-3.0-only

use thiserror::Error;

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
}

pub type ContextResult<T> = std::result::Result<T, ContextError>;
