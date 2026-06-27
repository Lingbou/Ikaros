// SPDX-License-Identifier: GPL-3.0-only

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use super::parse::HunkLine;
use super::stage::StagedFilePatch;

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

pub(super) fn build_patch_report(staged: &[StagedFilePatch]) -> PatchApplyReport {
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
