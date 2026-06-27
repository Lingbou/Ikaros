// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn resolve_manifest_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("plugin.toml")
    } else {
        path.to_path_buf()
    }
}

pub(super) fn copy_plugin_dir(source_dir: &Path, target_dir: &Path) -> Result<()> {
    reject_temp_path(source_dir, "plugin source path")?;
    fs::create_dir_all(target_dir).map_err(|source| IkarosError::io(target_dir, source))?;
    for entry in fs::read_dir(source_dir).map_err(|source| IkarosError::io(source_dir, source))? {
        let entry = entry.map_err(|source| IkarosError::io(source_dir, source))?;
        let source_path = entry.path();
        reject_temp_path(&source_path, "plugin source path")?;
        let target_path = target_dir.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)
            .map_err(|source| IkarosError::io(&source_path, source))?;
        if metadata.file_type().is_symlink() {
            return Err(IkarosError::Message(format!(
                "plugin install rejects symlinks: {}",
                source_path.display()
            )));
        }
        if metadata.is_dir() {
            copy_plugin_dir(&source_path, &target_path)?;
        } else if metadata.is_file() {
            copy_regular_file(&source_path, &target_path)?;
        }
    }
    Ok(())
}

pub(super) fn copy_regular_file(source_path: &Path, target_path: &Path) -> Result<()> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent).map_err(|source| IkarosError::io(parent, source))?;
    }
    fs::copy(source_path, target_path).map_err(|source| IkarosError::io(source_path, source))?;
    Ok(())
}

pub(super) fn remove_existing_target(target_dir: &Path) -> Result<()> {
    let metadata =
        fs::symlink_metadata(target_dir).map_err(|source| IkarosError::io(target_dir, source))?;
    if metadata.file_type().is_symlink() {
        return Err(IkarosError::Message(format!(
            "plugin management rejects symlink targets: {}",
            target_dir.display()
        )));
    }
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(target_dir).map_err(|source| IkarosError::io(target_dir, source))?;
    } else {
        fs::remove_file(target_dir).map_err(|source| IkarosError::io(target_dir, source))?;
    }
    Ok(())
}

pub(super) fn removal_target_for_manifest(
    skills_dir: &Path,
    manifest_path: &Path,
) -> Result<PathBuf> {
    let target = if manifest_path
        .file_name()
        .is_some_and(|name| name == "plugin.toml")
    {
        manifest_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| manifest_path.to_path_buf())
    } else {
        manifest_path.to_path_buf()
    };
    reject_temp_path(&target, "plugin uninstall target")?;
    ensure_child_path(skills_dir, &target)?;
    Ok(target)
}

pub(super) fn ensure_install_target_path(skills_dir: &Path, target_dir: &Path) -> Result<()> {
    reject_temp_path(target_dir, "plugin install target")?;
    let skills_dir =
        fs::canonicalize(skills_dir).map_err(|source| IkarosError::io(skills_dir, source))?;
    let target = if target_dir.exists() {
        fs::canonicalize(target_dir).map_err(|source| IkarosError::io(target_dir, source))?
    } else {
        let parent = target_dir.parent().ok_or_else(|| {
            IkarosError::Message(format!(
                "plugin install target has no parent: {}",
                target_dir.display()
            ))
        })?;
        let parent = fs::canonicalize(parent).map_err(|source| IkarosError::io(parent, source))?;
        let name = target_dir.file_name().ok_or_else(|| {
            IkarosError::Message(format!(
                "plugin install target has no final component: {}",
                target_dir.display()
            ))
        })?;
        parent.join(name)
    };
    if target == skills_dir || !target.starts_with(&skills_dir) {
        return Err(IkarosError::Message(format!(
            "plugin install target is outside the skills directory: {}",
            target.display()
        )));
    }
    Ok(())
}

pub(super) fn reject_self_replacement(source_path: &Path, target_dir: &Path) -> Result<()> {
    let source =
        fs::canonicalize(source_path).map_err(|source| IkarosError::io(source_path, source))?;
    let target =
        fs::canonicalize(target_dir).map_err(|source| IkarosError::io(target_dir, source))?;
    if source == target {
        return Err(IkarosError::Message(format!(
            "cannot replace plugin from its installed target: {}",
            target_dir.display()
        )));
    }
    Ok(())
}

pub(super) fn reject_temp_path(path: &Path, label: &str) -> Result<()> {
    if path
        .components()
        .any(|component| component.as_os_str() == ".temp")
    {
        return Err(IkarosError::Message(format!(
            "{label} must not target .temp"
        )));
    }
    Ok(())
}

fn ensure_child_path(parent: &Path, child: &Path) -> Result<()> {
    let parent = fs::canonicalize(parent).map_err(|source| IkarosError::io(parent, source))?;
    let child = fs::canonicalize(child).map_err(|source| IkarosError::io(child, source))?;
    if !child.starts_with(&parent) || child == parent {
        return Err(IkarosError::Message(format!(
            "plugin target is outside the skills directory: {}",
            child.display()
        )));
    }
    Ok(())
}
