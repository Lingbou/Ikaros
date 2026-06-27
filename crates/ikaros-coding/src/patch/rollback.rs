// SPDX-License-Identifier: GPL-3.0-only

#[cfg(test)]
use std::fs;

use ikaros_sandbox::FileSystem as ExecutionFileSystem;

use super::report::PatchFileOperation;
use super::stage::StagedFilePatch;

#[cfg(test)]
pub(super) fn rollback_staged_writes(written: &[StagedFilePatch]) {
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

pub(super) async fn rollback_staged_writes_with_env(
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
