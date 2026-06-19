// SPDX-License-Identifier: GPL-3.0-only

use crate::{PatchApplyReport, PatchFileChange, PatchFileOperation};
use ikaros_core::{IkarosError, Result, redact_secrets};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnDiffSummary {
    pub files_changed: usize,
    pub files_created: usize,
    pub files_deleted: usize,
    pub files_moved: usize,
    pub insertions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnDiffFile {
    pub path: PathBuf,
    pub status: TurnDiffFileStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old_content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnDiffFileStatus {
    Added,
    Modified,
    Deleted,
    Moved,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnifiedDiffRender {
    pub diff: String,
    pub summary: TurnDiffSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnDiffTracker {
    workspace_root: PathBuf,
    files: BTreeMap<PathBuf, TurnDiffFile>,
}

impl TurnDiffTracker {
    pub fn new(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            files: BTreeMap::new(),
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn files(&self) -> Vec<TurnDiffFile> {
        self.files.values().cloned().collect()
    }

    pub fn summary(&self) -> TurnDiffSummary {
        let files = self.files.values().collect::<Vec<_>>();
        TurnDiffSummary {
            files_changed: files.len(),
            files_created: files
                .iter()
                .filter(|file| file.status == TurnDiffFileStatus::Added)
                .count(),
            files_deleted: files
                .iter()
                .filter(|file| file.status == TurnDiffFileStatus::Deleted)
                .count(),
            files_moved: files
                .iter()
                .filter(|file| file.status == TurnDiffFileStatus::Moved)
                .count(),
            insertions: files
                .iter()
                .map(|file| line_count(file.new_content.as_deref()))
                .sum(),
            deletions: files
                .iter()
                .map(|file| line_count(file.old_content.as_deref()))
                .sum(),
        }
    }

    pub fn track_patch_report(&mut self, report: &PatchApplyReport) -> Result<()> {
        for change in &report.file_changes {
            self.track_patch_change(change)?;
        }
        Ok(())
    }

    pub fn track_patch_change(&mut self, change: &PatchFileChange) -> Result<()> {
        let file = turn_diff_file_from_patch_change(change)?;
        self.files.insert(file.path.clone(), file);
        Ok(())
    }

    pub fn render_unified_diff(&self) -> Option<UnifiedDiffRender> {
        let mut diff = String::new();
        for file in self.files.values() {
            render_file_diff(&mut diff, file);
        }
        (!diff.is_empty()).then(|| UnifiedDiffRender {
            diff: redact_secrets(&diff),
            summary: self.summary(),
        })
    }

    pub fn unified_diff(&self) -> Option<String> {
        self.render_unified_diff().map(|render| render.diff)
    }
}

fn turn_diff_file_from_patch_change(change: &PatchFileChange) -> Result<TurnDiffFile> {
    let status = match &change.operation {
        PatchFileOperation::Add => TurnDiffFileStatus::Added,
        PatchFileOperation::Update => TurnDiffFileStatus::Modified,
        PatchFileOperation::Delete => TurnDiffFileStatus::Deleted,
        PatchFileOperation::Move { .. } => TurnDiffFileStatus::Moved,
    };
    let old_path = match &change.operation {
        PatchFileOperation::Move { from } => Some(from.clone()),
        _ => None,
    };
    if status == TurnDiffFileStatus::Deleted && change.old_content.is_none() {
        return Err(IkarosError::Message(format!(
            "delete change missing old content for {}",
            change.path.display()
        )));
    }
    if matches!(
        status,
        TurnDiffFileStatus::Added | TurnDiffFileStatus::Modified | TurnDiffFileStatus::Moved
    ) && change.new_content.is_none()
    {
        return Err(IkarosError::Message(format!(
            "non-delete change missing new content for {}",
            change.path.display()
        )));
    }
    Ok(TurnDiffFile {
        path: change.path.clone(),
        status,
        old_path,
        old_content: change.old_content.clone(),
        new_content: change.new_content.clone(),
    })
}

fn render_file_diff(output: &mut String, file: &TurnDiffFile) {
    let old_path = file.old_path.as_ref().unwrap_or(&file.path);
    output.push_str(&format!(
        "diff --git a/{} b/{}\n",
        display_path(old_path),
        display_path(&file.path)
    ));
    if file.status == TurnDiffFileStatus::Moved {
        output.push_str(&format!("rename from {}\n", display_path(old_path)));
        output.push_str(&format!("rename to {}\n", display_path(&file.path)));
    }
    match file.status {
        TurnDiffFileStatus::Added => {
            output.push_str("--- /dev/null\n");
            output.push_str(&format!("+++ b/{}\n", display_path(&file.path)));
        }
        TurnDiffFileStatus::Deleted => {
            output.push_str(&format!("--- a/{}\n", display_path(old_path)));
            output.push_str("+++ /dev/null\n");
        }
        TurnDiffFileStatus::Modified | TurnDiffFileStatus::Moved => {
            output.push_str(&format!("--- a/{}\n", display_path(old_path)));
            output.push_str(&format!("+++ b/{}\n", display_path(&file.path)));
        }
    }
    let old_lines = content_lines(file.old_content.as_deref());
    let new_lines = content_lines(file.new_content.as_deref());
    output.push_str(&format!(
        "@@ -1,{} +1,{} @@\n",
        old_lines.len(),
        new_lines.len()
    ));
    for line in old_lines {
        output.push('-');
        output.push_str(&line);
        output.push('\n');
    }
    for line in new_lines {
        output.push('+');
        output.push_str(&line);
        output.push('\n');
    }
}

fn content_lines(content: Option<&str>) -> Vec<String> {
    content
        .unwrap_or_default()
        .lines()
        .map(ToOwned::to_owned)
        .collect()
}

fn line_count(content: Option<&str>) -> usize {
    content_lines(content).len()
}

fn display_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}
