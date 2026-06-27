// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use ikaros_sandbox::FileSystem as ExecutionFileSystem;
use std::path::{Path, PathBuf};

use super::apply::apply_file_patch;
use super::parse::{FilePatch, parse_unified_diff};
use super::path::{path_metadata_with_env, resolve_patch_path_with_env};
use super::report::PatchFileOperation;

#[derive(Debug, Clone)]
pub(super) struct StagedFilePatch {
    pub(super) source: PathBuf,
    pub(super) target: PathBuf,
    pub(super) existed: bool,
    pub(super) original: String,
    pub(super) updated: Option<String>,
    pub(super) operation: PatchFileOperation,
    pub(super) patch: FilePatch,
}

pub(super) async fn stage_unified_diff_with_env(
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
