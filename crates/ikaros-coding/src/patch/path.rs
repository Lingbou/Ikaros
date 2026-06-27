// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use ikaros_sandbox::{FileMetadata, FileSystem as ExecutionFileSystem};
use std::{
    ffi::OsString,
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

pub(super) fn validate_relative_patch_path(path: &Path) -> Result<()> {
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

#[cfg(test)]
pub(super) fn resolve_patch_path(root: &Path, relative: &Path) -> Result<PathBuf> {
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

pub(super) async fn resolve_patch_path_with_env(
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

pub(super) async fn path_metadata_with_env(
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
