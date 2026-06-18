// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use ikaros_harness::{FileMetadata, FileSystem as ExecutionFileSystem};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchApplyReport {
    pub files_changed: usize,
    pub files_created: usize,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub files_deleted: usize,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub files_moved: usize,
    pub hunks_applied: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub paths: Vec<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_changes: Vec<PatchFileChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchFileChange {
    pub path: PathBuf,
    pub operation: PatchFileOperation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PatchFileOperation {
    Add,
    Update,
    Delete,
    Move { from: PathBuf },
}

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

pub struct GuardedPatchApplier;

impl GuardedPatchApplier {
    #[cfg(test)]
    pub fn apply_unified_diff_checked(
        root: &Path,
        diff: &str,
    ) -> std::result::Result<PatchApplyReport, PatchFailure> {
        Self::apply_unified_diff(root, diff).map_err(PatchFailure::from_error)
    }

    pub(crate) fn apply_unified_diff(root: &Path, diff: &str) -> Result<PatchApplyReport> {
        let patches = parse_unified_diff(diff)?;
        if patches.is_empty() {
            return Err(IkarosError::Message(
                "diff did not contain any file hunks".into(),
            ));
        }

        let mut staged = Vec::<StagedFilePatch>::new();

        for patch in patches {
            let (source_relative, target_relative) = patch.source_and_target_paths()?;
            let source = resolve_patch_path(root, source_relative)?;
            let target = resolve_patch_path(root, target_relative)?;
            if staged
                .iter()
                .any(|staged_patch| staged_patch.target == target || staged_patch.source == target)
            {
                return Err(IkarosError::Message(format!(
                    "guarded edit contains duplicate target: {}",
                    target_relative.display()
                )));
            }
            let operation = patch.operation()?;
            if matches!(operation, PatchFileOperation::Add) && target.exists() {
                return Err(IkarosError::Message(format!(
                    "guarded edit add target already exists: {}",
                    target_relative.display()
                )));
            }
            if matches!(operation, PatchFileOperation::Move { .. }) && target.exists() {
                return Err(IkarosError::Message(format!(
                    "guarded edit move target already exists: {}",
                    target_relative.display()
                )));
            }
            let existed = !matches!(operation, PatchFileOperation::Add) && source.exists();
            let original = if existed {
                fs::read_to_string(&source).map_err(|error| IkarosError::io(&source, error))?
            } else {
                String::new()
            };
            let updated = if matches!(operation, PatchFileOperation::Delete) {
                apply_file_patch(&original, &patch)?;
                None
            } else {
                Some(apply_file_patch(&original, &patch)?)
            };
            staged.push(StagedFilePatch {
                source,
                target,
                existed,
                original,
                updated,
                operation,
                patch,
            });
        }

        let report = build_patch_report(&staged);
        let mut written = Vec::<StagedFilePatch>::new();
        for staged_patch in staged {
            if let Some(parent) = staged_patch.target.parent()
                && staged_patch.updated.is_some()
            {
                if let Err(source) = fs::create_dir_all(parent) {
                    rollback_staged_writes(&written);
                    return Err(IkarosError::io(parent, source));
                }
            }
            match &staged_patch.updated {
                Some(updated) => {
                    if let Err(source) = fs::write(&staged_patch.target, updated) {
                        let target = staged_patch.target.clone();
                        rollback_staged_writes(&written);
                        return Err(IkarosError::io(&target, source));
                    }
                    written.push(staged_patch.clone());
                    if staged_patch.source != staged_patch.target
                        && let Err(source) = fs::remove_file(&staged_patch.source)
                    {
                        let source_path = staged_patch.source.clone();
                        rollback_staged_writes(&written);
                        return Err(IkarosError::io(&source_path, source));
                    }
                }
                None => {
                    if let Err(source) = fs::remove_file(&staged_patch.source) {
                        let source_path = staged_patch.source.clone();
                        rollback_staged_writes(&written);
                        return Err(IkarosError::io(&source_path, source));
                    }
                    written.push(staged_patch.clone());
                }
            }
        }

        Ok(report)
    }

    pub async fn apply_unified_diff_with_env(
        root: &Path,
        diff: &str,
        file_system: &dyn ExecutionFileSystem,
    ) -> Result<PatchApplyReport> {
        let staged = stage_unified_diff_with_env(root, diff, file_system).await?;
        let report = build_patch_report(&staged);
        let mut written = Vec::<StagedFilePatch>::new();
        for staged_patch in staged {
            if let Some(parent) = staged_patch.target.parent()
                && staged_patch.updated.is_some()
                && let Err(error) = file_system.create_dir_all(parent).await
            {
                rollback_staged_writes_with_env(file_system, &written).await;
                return Err(error);
            }
            match &staged_patch.updated {
                Some(updated) => {
                    if let Err(error) = file_system
                        .write_string(&staged_patch.target, updated.clone())
                        .await
                    {
                        rollback_staged_writes_with_env(file_system, &written).await;
                        return Err(error);
                    }
                    written.push(staged_patch.clone());
                    if staged_patch.source != staged_patch.target
                        && let Err(error) = file_system.remove_file(&staged_patch.source).await
                    {
                        rollback_staged_writes_with_env(file_system, &written).await;
                        return Err(error);
                    }
                }
                None => {
                    if let Err(error) = file_system.remove_file(&staged_patch.source).await {
                        rollback_staged_writes_with_env(file_system, &written).await;
                        return Err(error);
                    }
                    written.push(staged_patch.clone());
                }
            }
        }
        Ok(report)
    }

    pub async fn apply_unified_diff_with_env_checked(
        root: &Path,
        diff: &str,
        file_system: &dyn ExecutionFileSystem,
    ) -> std::result::Result<PatchApplyReport, PatchFailure> {
        Self::apply_unified_diff_with_env(root, diff, file_system)
            .await
            .map_err(PatchFailure::from_error)
    }
}

async fn stage_unified_diff_with_env(
    root: &Path,
    diff: &str,
    file_system: &dyn ExecutionFileSystem,
) -> Result<Vec<StagedFilePatch>> {
    let patches = parse_unified_diff(diff)?;
    if patches.is_empty() {
        return Err(IkarosError::Message(
            "diff did not contain any file hunks".into(),
        ));
    }

    let mut staged = Vec::<StagedFilePatch>::new();
    for patch in patches {
        let (source_relative, target_relative) = patch.source_and_target_paths()?;
        let source = resolve_patch_path_with_env(root, source_relative, file_system).await?;
        let target = resolve_patch_path_with_env(root, target_relative, file_system).await?;
        if staged
            .iter()
            .any(|staged_patch| staged_patch.target == target || staged_patch.source == target)
        {
            return Err(IkarosError::Message(format!(
                "guarded edit contains duplicate target: {}",
                target_relative.display()
            )));
        }
        let operation = patch.operation()?;
        if matches!(operation, PatchFileOperation::Add)
            && path_metadata_with_env(file_system, &target)
                .await?
                .is_some()
        {
            return Err(IkarosError::Message(format!(
                "guarded edit add target already exists: {}",
                target_relative.display()
            )));
        }
        if matches!(operation, PatchFileOperation::Move { .. })
            && path_metadata_with_env(file_system, &target)
                .await?
                .is_some()
        {
            return Err(IkarosError::Message(format!(
                "guarded edit move target already exists: {}",
                target_relative.display()
            )));
        }
        let existed = !matches!(operation, PatchFileOperation::Add)
            && path_metadata_with_env(file_system, &source)
                .await?
                .is_some();
        let original = if existed {
            file_system.read_to_string(&source).await?
        } else {
            String::new()
        };
        let updated = if matches!(operation, PatchFileOperation::Delete) {
            apply_file_patch(&original, &patch)?;
            None
        } else {
            Some(apply_file_patch(&original, &patch)?)
        };
        staged.push(StagedFilePatch {
            source,
            target,
            existed,
            original,
            updated,
            operation,
            patch,
        });
    }
    Ok(staged)
}

#[derive(Debug, Clone)]
struct StagedFilePatch {
    source: PathBuf,
    target: PathBuf,
    existed: bool,
    original: String,
    updated: Option<String>,
    operation: PatchFileOperation,
    patch: FilePatch,
}

fn build_patch_report(staged: &[StagedFilePatch]) -> PatchApplyReport {
    let mut report = PatchApplyReport {
        files_changed: 0,
        files_created: 0,
        files_deleted: 0,
        files_moved: 0,
        hunks_applied: 0,
        insertions: 0,
        deletions: 0,
        paths: Vec::new(),
        file_changes: Vec::new(),
    };
    for staged_patch in staged {
        let patch = &staged_patch.patch;
        report.files_changed += 1;
        match staged_patch.operation {
            PatchFileOperation::Add => report.files_created += 1,
            PatchFileOperation::Delete => report.files_deleted += 1,
            PatchFileOperation::Move { .. } => report.files_moved += 1,
            PatchFileOperation::Update => {}
        }
        if !staged_patch.existed && matches!(staged_patch.operation, PatchFileOperation::Update) {
            report.files_created += 1;
        }
        report.hunks_applied += patch.hunks.len();
        report.insertions += patch
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| matches!(line, HunkLine::Add(_)))
            .count();
        report.deletions += patch
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .filter(|line| matches!(line, HunkLine::Remove(_)))
            .count();
        report.paths.push(staged_patch.target.clone());
        report.file_changes.push(PatchFileChange {
            path: patch
                .new_path
                .clone()
                .or_else(|| patch.old_path.clone())
                .unwrap_or_else(|| staged_patch.target.clone()),
            operation: staged_patch.operation.clone(),
            old_content: staged_patch.existed.then(|| staged_patch.original.clone()),
            new_content: staged_patch.updated.clone(),
        });
    }
    report
}

fn is_zero_usize(value: &usize) -> bool {
    *value == 0
}

fn rollback_staged_writes(written: &[StagedFilePatch]) {
    for staged_patch in written.iter().rev() {
        match &staged_patch.operation {
            PatchFileOperation::Add => {
                let _ = fs::remove_file(&staged_patch.target);
            }
            PatchFileOperation::Update => {
                if staged_patch.existed {
                    let _ = fs::write(&staged_patch.target, &staged_patch.original);
                } else {
                    let _ = fs::remove_file(&staged_patch.target);
                }
            }
            PatchFileOperation::Delete => {
                if staged_patch.existed {
                    let _ = fs::write(&staged_patch.source, &staged_patch.original);
                }
            }
            PatchFileOperation::Move { .. } => {
                let _ = fs::remove_file(&staged_patch.target);
                if staged_patch.existed {
                    let _ = fs::write(&staged_patch.source, &staged_patch.original);
                }
            }
        }
    }
}

async fn rollback_staged_writes_with_env(
    file_system: &dyn ExecutionFileSystem,
    written: &[StagedFilePatch],
) {
    for staged_patch in written.iter().rev() {
        match &staged_patch.operation {
            PatchFileOperation::Add => {
                let _ = file_system.remove_file(&staged_patch.target).await;
            }
            PatchFileOperation::Update => {
                if staged_patch.existed {
                    let _ = file_system
                        .write_string(&staged_patch.target, staged_patch.original.clone())
                        .await;
                } else {
                    let _ = file_system.remove_file(&staged_patch.target).await;
                }
            }
            PatchFileOperation::Delete => {
                if staged_patch.existed {
                    let _ = file_system
                        .write_string(&staged_patch.source, staged_patch.original.clone())
                        .await;
                }
            }
            PatchFileOperation::Move { .. } => {
                let _ = file_system.remove_file(&staged_patch.target).await;
                if staged_patch.existed {
                    let _ = file_system
                        .write_string(&staged_patch.source, staged_patch.original.clone())
                        .await;
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FilePatch {
    old_path: Option<PathBuf>,
    new_path: Option<PathBuf>,
    hunks: Vec<Hunk>,
}

impl FilePatch {
    fn source_and_target_paths(&self) -> Result<(&Path, &Path)> {
        let source = self
            .old_path
            .as_ref()
            .or(self.new_path.as_ref())
            .ok_or_else(|| {
                IkarosError::Message("file patch missing source and target path".into())
            })?;
        let target = self
            .new_path
            .as_ref()
            .or(self.old_path.as_ref())
            .ok_or_else(|| {
                IkarosError::Message("file patch missing source and target path".into())
            })?;
        Ok((source.as_path(), target.as_path()))
    }

    fn operation(&self) -> Result<PatchFileOperation> {
        match (&self.old_path, &self.new_path) {
            (None, Some(_)) => Ok(PatchFileOperation::Add),
            (Some(_), None) => Ok(PatchFileOperation::Delete),
            (Some(old), Some(new)) if old == new => Ok(PatchFileOperation::Update),
            (Some(old), Some(_)) => Ok(PatchFileOperation::Move { from: old.clone() }),
            (None, None) => Err(IkarosError::Message(
                "file patch missing source and target path".into(),
            )),
        }
    }

    fn display_path(&self) -> PathBuf {
        self.new_path
            .clone()
            .or_else(|| self.old_path.clone())
            .unwrap_or_else(|| PathBuf::from("<unknown>"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Hunk {
    old_start: usize,
    lines: Vec<HunkLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum HunkLine {
    Context(String),
    Add(String),
    Remove(String),
}

fn parse_unified_diff(diff: &str) -> Result<Vec<FilePatch>> {
    let mut patches = Vec::<FilePatch>::new();
    let mut current_old_path = None::<PathBuf>;
    let mut current_new_path = None::<PathBuf>;
    let mut current_hunks = Vec::<Hunk>::new();
    let mut current_hunk = None::<Hunk>;

    for raw_line in diff.lines() {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line.starts_with("diff --git ") {
            flush_hunk(&mut current_hunks, &mut current_hunk);
            flush_file(
                &mut patches,
                &mut current_old_path,
                &mut current_new_path,
                &mut current_hunks,
            )?;
            continue;
        }
        if let Some(path) = line.strip_prefix("rename from ") {
            current_old_path = Some(parse_diff_path(path)?);
            continue;
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            current_new_path = Some(parse_diff_path(path)?);
            continue;
        }
        if let Some(path) = line.strip_prefix("--- ") {
            flush_hunk(&mut current_hunks, &mut current_hunk);
            current_old_path = parse_optional_diff_path(path)?;
            continue;
        }
        if let Some(path) = line.strip_prefix("+++ ") {
            flush_hunk(&mut current_hunks, &mut current_hunk);
            current_new_path = parse_optional_diff_path(path)?;
            continue;
        }
        if let Some(header) = line.strip_prefix("@@ ") {
            flush_hunk(&mut current_hunks, &mut current_hunk);
            current_hunk = Some(Hunk {
                old_start: parse_hunk_old_start(header)?,
                lines: Vec::new(),
            });
            continue;
        }
        if line == "\\ No newline at end of file" {
            continue;
        }
        if let Some(hunk) = &mut current_hunk {
            let Some(marker) = line.chars().next() else {
                return Err(IkarosError::Message("empty hunk line is invalid".into()));
            };
            let text = line
                .get(marker.len_utf8()..)
                .ok_or_else(|| IkarosError::Message("invalid hunk line".into()))?
                .to_string();
            match marker {
                ' ' => hunk.lines.push(HunkLine::Context(text)),
                '+' => hunk.lines.push(HunkLine::Add(text)),
                '-' => hunk.lines.push(HunkLine::Remove(text)),
                _ => {
                    return Err(IkarosError::Message(format!(
                        "unsupported hunk line marker: {marker}"
                    )));
                }
            }
        }
    }

    flush_hunk(&mut current_hunks, &mut current_hunk);
    flush_file(
        &mut patches,
        &mut current_old_path,
        &mut current_new_path,
        &mut current_hunks,
    )?;
    Ok(patches)
}

fn flush_hunk(hunks: &mut Vec<Hunk>, hunk: &mut Option<Hunk>) {
    if let Some(hunk) = hunk.take() {
        hunks.push(hunk);
    }
}

fn flush_file(
    patches: &mut Vec<FilePatch>,
    old_path: &mut Option<PathBuf>,
    new_path: &mut Option<PathBuf>,
    hunks: &mut Vec<Hunk>,
) -> Result<()> {
    if hunks.is_empty() && old_path.as_ref() == new_path.as_ref() {
        *old_path = None;
        *new_path = None;
        return Ok(());
    }
    if old_path.is_none() && new_path.is_none() {
        return Err(IkarosError::Message(
            "file patch missing source and target path".into(),
        ));
    }
    patches.push(FilePatch {
        old_path: old_path.take(),
        new_path: new_path.take(),
        hunks: std::mem::take(hunks),
    });
    Ok(())
}

pub(crate) fn parse_diff_path(raw: &str) -> Result<PathBuf> {
    parse_optional_diff_path(raw)?.ok_or_else(|| {
        IkarosError::Message("delete-only patches are not supported by this path parser".into())
    })
}

fn parse_optional_diff_path(raw: &str) -> Result<Option<PathBuf>> {
    let trimmed = raw.trim();
    if trimmed.contains('"') || (trimmed.contains(' ') && !trimmed.contains('\t')) {
        return Err(IkarosError::Message(format!(
            "patch path contains unsupported quoted or space-delimited form: {trimmed}"
        )));
    }
    let path = trimmed.split_whitespace().next().unwrap_or(trimmed);
    if path == "/dev/null" {
        return Ok(None);
    }
    let stripped = path
        .strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path);
    let path = PathBuf::from(stripped);
    validate_relative_patch_path(&path)?;
    Ok(Some(path))
}

fn parse_hunk_old_start(header: &str) -> Result<usize> {
    let mut parts = header.split_whitespace();
    let old_range = parts
        .next()
        .ok_or_else(|| IkarosError::Message("hunk header missing old range".into()))?;
    let new_range = parts
        .next()
        .ok_or_else(|| IkarosError::Message("hunk header missing new range".into()))?;
    let old_start = parse_hunk_range_start(old_range, '-')?;
    parse_hunk_range_start(new_range, '+')?;
    Ok(old_start)
}

fn parse_hunk_range_start(range: &str, marker: char) -> Result<usize> {
    let range = range
        .strip_prefix(marker)
        .ok_or_else(|| IkarosError::Message(format!("hunk range must start with '{marker}'")))?;
    let start = range
        .split(',')
        .next()
        .ok_or_else(|| IkarosError::Message("hunk range missing start".into()))?;
    let length = range.split(',').nth(1);
    let parsed_start = start
        .parse::<usize>()
        .map_err(|source| IkarosError::Message(format!("invalid hunk range start: {source}")))?;
    if let Some(length) = length {
        length.parse::<usize>().map_err(|source| {
            IkarosError::Message(format!("invalid hunk range length: {source}"))
        })?;
    }
    Ok(parsed_start)
}

fn resolve_patch_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    validate_relative_patch_path(relative)?;
    let root = fs::canonicalize(root).map_err(|source| IkarosError::io(root, source))?;
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => Ok(name.to_os_string()),
            _ => Err(IkarosError::Message(format!(
                "patch path contains unsupported component: {}",
                relative.display()
            ))),
        })
        .collect::<Result<Vec<OsString>>>()?;

    let mut target = root.clone();
    for (index, component) in components.iter().enumerate() {
        target.push(component);
        let is_leaf = index + 1 == components.len();
        match fs::symlink_metadata(&target) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(IkarosError::Message(format!(
                        "guarded edit rejects symlink patch target: {}",
                        target.display()
                    )));
                }
                if !is_leaf && !metadata.is_dir() {
                    return Err(IkarosError::Message(format!(
                        "patch path parent is not a directory: {}",
                        target.display()
                    )));
                }
                let canonical =
                    fs::canonicalize(&target).map_err(|source| IkarosError::io(&target, source))?;
                if !canonical.starts_with(&root) {
                    return Err(IkarosError::Message(format!(
                        "patch path escapes workspace: {}",
                        target.display()
                    )));
                }
            }
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(IkarosError::io(&target, source)),
        }
    }

    Ok(target)
}

async fn resolve_patch_path_with_env(
    root: &Path,
    relative: &Path,
    file_system: &dyn ExecutionFileSystem,
) -> Result<PathBuf> {
    validate_relative_patch_path(relative)?;
    let root = fs::canonicalize(root).map_err(|source| IkarosError::io(root, source))?;
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => Ok(name.to_os_string()),
            _ => Err(IkarosError::Message(format!(
                "patch path contains unsupported component: {}",
                relative.display()
            ))),
        })
        .collect::<Result<Vec<OsString>>>()?;

    let mut target = root.clone();
    for (index, component) in components.iter().enumerate() {
        target.push(component);
        let is_leaf = index + 1 == components.len();
        if let Some(metadata) = path_metadata_with_env(file_system, &target).await? {
            if metadata.is_symlink {
                return Err(IkarosError::Message(format!(
                    "guarded edit rejects symlink patch target: {}",
                    target.display()
                )));
            }
            if !is_leaf && !metadata.is_dir {
                return Err(IkarosError::Message(format!(
                    "patch path parent is not a directory: {}",
                    target.display()
                )));
            }
        }
    }

    Ok(target)
}

async fn path_metadata_with_env(
    file_system: &dyn ExecutionFileSystem,
    path: &Path,
) -> Result<Option<FileMetadata>> {
    match file_system.path_metadata(path).await {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if is_not_found(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn is_not_found(error: &IkarosError) -> bool {
    matches!(error, IkarosError::Io { source, .. } if source.kind() == ErrorKind::NotFound)
}

fn validate_relative_patch_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(IkarosError::Message(format!(
            "patch path must be relative: {}",
            path.display()
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(name) if name == ".temp" => {
                return Err(IkarosError::Message(
                    "guarded edit cannot modify .temp".into(),
                ));
            }
            Component::Normal(_) => {}
            _ => {
                return Err(IkarosError::Message(format!(
                    "patch path contains unsupported component: {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn apply_file_patch(original: &str, patch: &FilePatch) -> Result<String> {
    let source = original.lines().map(ToOwned::to_owned).collect::<Vec<_>>();
    let mut output = Vec::<String>::new();
    let mut source_index = 0usize;

    for hunk in &patch.hunks {
        let hunk_start = hunk.old_start.saturating_sub(1);
        if hunk_start < source_index || hunk_start > source.len() {
            return Err(IkarosError::Message(format!(
                "hunk for {} is out of range",
                patch.display_path().display()
            )));
        }
        let anchor = hunk_source_anchor(hunk);
        let candidate_lines = find_line_sequence(&source, &anchor);
        if candidate_lines.len() > 1 {
            return Err(IkarosError::Message(format!(
                "ambiguous hunk anchor in {}: candidate source lines: {}",
                patch.display_path().display(),
                candidate_lines
                    .iter()
                    .map(|line| line.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        }
        output.extend(source[source_index..hunk_start].iter().cloned());
        source_index = hunk_start;

        for line in &hunk.lines {
            match line {
                HunkLine::Context(text) => {
                    expect_source_line(&source, source_index, text, hunk, &patch.display_path())?;
                    output.push(text.clone());
                    source_index += 1;
                }
                HunkLine::Remove(text) => {
                    expect_source_line(&source, source_index, text, hunk, &patch.display_path())?;
                    source_index += 1;
                }
                HunkLine::Add(text) => output.push(text.clone()),
            }
        }
    }

    output.extend(source[source_index..].iter().cloned());
    let mut rendered = output.join("\n");
    if !rendered.is_empty() {
        rendered.push('\n');
    }
    Ok(rendered)
}

fn expect_source_line(
    source: &[String],
    index: usize,
    expected: &str,
    hunk: &Hunk,
    path: &Path,
) -> Result<()> {
    let actual = source.get(index).ok_or_else(|| {
        IkarosError::Message(format!(
            "hunk for {} expected more source lines",
            path.display()
        ))
    })?;
    if actual != expected {
        let hunk_start = hunk.old_start.saturating_sub(1);
        let replacement = hunk_replacement_anchor(hunk);
        if sequence_matches_at(source, hunk_start, &replacement) {
            return Err(IkarosError::Message(format!(
                "hunk already applied in {} at source line {}",
                path.display(),
                index + 1
            )));
        }
        return Err(IkarosError::Message(format!(
            "hunk mismatch in {} at source line {}: expected `{}`, actual `{}`",
            path.display(),
            index + 1,
            expected.replace('`', "\\`"),
            actual.replace('`', "\\`")
        )));
    }
    Ok(())
}

fn hunk_source_anchor(hunk: &Hunk) -> Vec<String> {
    hunk.lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Context(text) | HunkLine::Remove(text) => Some(text.clone()),
            HunkLine::Add(_) => None,
        })
        .collect()
}

fn hunk_replacement_anchor(hunk: &Hunk) -> Vec<String> {
    hunk.lines
        .iter()
        .filter_map(|line| match line {
            HunkLine::Context(text) | HunkLine::Add(text) => Some(text.clone()),
            HunkLine::Remove(_) => None,
        })
        .collect()
}

fn find_line_sequence(source: &[String], needle: &[String]) -> Vec<usize> {
    if needle.is_empty() || needle.len() > source.len() {
        return Vec::new();
    }
    source
        .windows(needle.len())
        .enumerate()
        .filter_map(|(index, window)| (window == needle).then_some(index + 1))
        .collect()
}

fn sequence_matches_at(source: &[String], start: usize, needle: &[String]) -> bool {
    if needle.is_empty() || start + needle.len() > source.len() {
        return false;
    }
    source[start..start + needle.len()] == *needle
}
