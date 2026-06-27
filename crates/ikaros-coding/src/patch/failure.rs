// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::IkarosError;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchFailure {
    pub kind: PatchFailureKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    pub message: Box<str>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_line: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected: Option<Box<str>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual: Option<Box<str>>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub already_applied: bool,
    #[serde(default, skip_serializing_if = "is_empty_usize_slice")]
    pub candidate_lines: Box<[usize]>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PatchFailureKind {
    EmptyDiff,
    DuplicateTarget,
    PathRejected,
    AlreadyApplied,
    AmbiguousAnchor,
    HunkOutOfRange,
    HunkMismatch,
    Io,
    Unsupported,
}

impl PatchFailure {
    fn new(kind: PatchFailureKind, path: Option<PathBuf>, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            kind,
            path,
            message: message.into_boxed_str(),
            source_line: None,
            expected: None,
            actual: None,
            already_applied: false,
            candidate_lines: Box::new([]),
        }
    }

    fn with_source_line(mut self, source_line: Option<usize>) -> Self {
        self.source_line = source_line;
        self
    }

    fn with_expected_actual(mut self, expected: Option<String>, actual: Option<String>) -> Self {
        self.expected = expected.map(String::into_boxed_str);
        self.actual = actual.map(String::into_boxed_str);
        self
    }

    fn with_already_applied(mut self) -> Self {
        self.already_applied = true;
        self
    }

    fn with_candidate_lines(mut self, candidate_lines: Vec<usize>) -> Self {
        self.candidate_lines = candidate_lines.into_boxed_slice();
        self
    }

    pub(crate) fn from_error(error: IkarosError) -> Self {
        classify_patch_error(error.to_string())
    }
}

fn classify_patch_error(message: String) -> PatchFailure {
    let lower = message.to_ascii_lowercase();
    let path = patch_path_from_message(&message);
    let kind = if lower.contains("did not contain any file hunks") {
        PatchFailureKind::EmptyDiff
    } else if lower.contains("duplicate target") || lower.contains("target already exists") {
        PatchFailureKind::DuplicateTarget
    } else if lower.contains("already applied") {
        PatchFailureKind::AlreadyApplied
    } else if lower.contains("ambiguous hunk anchor") {
        PatchFailureKind::AmbiguousAnchor
    } else if lower.contains("path")
        || lower.contains("symlink")
        || lower.contains("workspace")
        || lower.contains(".temp")
    {
        PatchFailureKind::PathRejected
    } else if lower.contains("out of range") || lower.contains("expected more source lines") {
        PatchFailureKind::HunkOutOfRange
    } else if lower.contains("hunk mismatch") {
        PatchFailureKind::HunkMismatch
    } else if lower.contains("io error") || lower.contains("failed to") {
        PatchFailureKind::Io
    } else {
        PatchFailureKind::Unsupported
    };
    let source_line = patch_source_line_from_message(&message);
    let (expected, actual) = patch_expected_actual_from_message(&message);
    let candidate_lines = patch_candidate_lines_from_message(&message);
    let failure = PatchFailure::new(kind, path, message)
        .with_source_line(source_line)
        .with_expected_actual(expected, actual)
        .with_candidate_lines(candidate_lines);
    if failure.kind == PatchFailureKind::AlreadyApplied {
        failure.with_already_applied()
    } else {
        failure
    }
}

fn patch_path_from_message(message: &str) -> Option<PathBuf> {
    let (_, rest) = message.split_once(" in ")?;
    let path = rest
        .split_once(" at ")
        .map(|(path, _)| path)
        .or_else(|| rest.split_once(':').map(|(path, _)| path))?;
    let path = path.trim();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn patch_source_line_from_message(message: &str) -> Option<usize> {
    let (_, rest) = message.split_once("source line ")?;
    rest.split(|ch: char| !ch.is_ascii_digit())
        .next()
        .and_then(|value| value.parse::<usize>().ok())
}

fn patch_expected_actual_from_message(message: &str) -> (Option<String>, Option<String>) {
    let expected = message
        .split_once("expected ")
        .and_then(|(_, rest)| rest.split_once(", actual ").map(|(value, _)| value))
        .map(unquote_patch_diagnostic);
    let actual = message
        .split_once(", actual ")
        .map(|(_, value)| unquote_patch_diagnostic(value));
    (expected, actual)
}

fn unquote_patch_diagnostic(value: &str) -> String {
    value
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .replace("\\n", "\n")
}

fn patch_candidate_lines_from_message(message: &str) -> Vec<usize> {
    let Some((_, rest)) = message.split_once("candidate source lines:") else {
        return Vec::new();
    };
    rest.split(',')
        .filter_map(|value| value.trim().parse::<usize>().ok())
        .collect()
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_empty_usize_slice(value: &[usize]) -> bool {
    value.is_empty()
}
