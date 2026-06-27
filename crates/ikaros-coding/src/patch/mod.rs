// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::Result;
use ikaros_sandbox::FileSystem as ExecutionFileSystem;
use std::path::Path;

mod apply;
mod failure;
mod parse;
mod path;
mod report;
mod rollback;
mod stage;

pub use failure::{PatchFailure, PatchFailureKind};
pub(crate) use parse::parse_diff_path;
pub use report::{PatchApplyReport, PatchFileChange, PatchFileOperation};

use report::build_patch_report;
use rollback::rollback_staged_writes_with_env;
use stage::{StagedFilePatch, stage_unified_diff_with_env};

#[cfg(test)]
use {
    apply::apply_file_patch, ikaros_core::IkarosError, parse::parse_unified_diff,
    path::resolve_patch_path, rollback::rollback_staged_writes, std::fs,
};

pub struct GuardedPatchApplier;

impl GuardedPatchApplier {
    #[cfg(test)]
    pub fn apply_unified_diff_checked(
        root: &Path,
        diff: &str,
    ) -> std::result::Result<PatchApplyReport, PatchFailure> {
        Self::apply_unified_diff(root, diff).map_err(PatchFailure::from_error)
    }

    #[cfg(test)]
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
