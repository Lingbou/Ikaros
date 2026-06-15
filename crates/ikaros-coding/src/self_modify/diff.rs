// SPDX-License-Identifier: GPL-3.0-only

use super::SelfModifyStore;
use crate::patch::parse_diff_path;
use ikaros_core::{IkarosError, Result};
use std::{
    fs,
    io::ErrorKind,
    path::{Component, Path, PathBuf},
};

impl SelfModifyStore {
    pub(super) fn resolve_target(&self, target_path: &Path) -> Result<PathBuf> {
        let workspace_root = fs::canonicalize(&self.workspace_root)
            .map_err(|source| IkarosError::io(&self.workspace_root, source))?;
        let absolute = normalize_path(&if target_path.is_absolute() {
            target_path.to_path_buf()
        } else {
            workspace_root.join(target_path)
        });
        if !absolute.starts_with(&workspace_root) {
            return Err(IkarosError::Message(
                "self-modify target must stay inside the workspace".into(),
            ));
        }
        let relative = absolute.strip_prefix(&workspace_root).map_err(|_| {
            IkarosError::Message("self-modify target must stay inside the workspace".into())
        })?;
        if relative.as_os_str().is_empty() {
            return Err(IkarosError::Message(
                "self-modify target must include a file name".into(),
            ));
        }
        let resolved = resolve_non_symlink_target(&workspace_root, relative)?;
        if resolved.file_name().is_none() {
            return Err(IkarosError::Message(
                "self-modify target must include a file name".into(),
            ));
        }
        if resolved
            .components()
            .any(|component| matches!(component, Component::Normal(name) if name == ".temp"))
        {
            return Err(IkarosError::Message(
                "self-modify target under .temp is denied".into(),
            ));
        }
        if !resolved.starts_with(&workspace_root) {
            return Err(IkarosError::Message(
                "self-modify target must stay inside the workspace".into(),
            ));
        }
        let store_dir = normalize_path(&if self.store_dir.is_absolute() {
            self.store_dir.clone()
        } else {
            workspace_root.join(&self.store_dir)
        });
        if resolved.starts_with(&store_dir) {
            return Err(IkarosError::Message(
                "self-modify store paths are protected".into(),
            ));
        }
        let rendered = resolved
            .to_string_lossy()
            .replace('\\', "/")
            .to_ascii_lowercase();
        for protected in ["/audit/", "/approvals", "/secrets/", "/self-modify/"] {
            if rendered.contains(protected) {
                return Err(IkarosError::Message(
                    "self-modify protected local state paths are denied".into(),
                ));
            }
        }
        if rendered.contains("secret") || rendered.contains("token") || rendered.contains("key") {
            return Err(IkarosError::Message(
                "secret-like self-modify target path is denied".into(),
            ));
        }
        Ok(resolved)
    }
}

fn resolve_non_symlink_target(workspace_root: &Path, relative: &Path) -> Result<PathBuf> {
    let mut resolved = workspace_root.to_path_buf();
    let components = relative.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        let name = match component {
            Component::Normal(name) => name,
            _ => {
                return Err(IkarosError::Message(format!(
                    "self-modify target contains unsupported component: {}",
                    relative.display()
                )));
            }
        };
        resolved.push(name);
        let is_leaf = index + 1 == components.len();
        match fs::symlink_metadata(&resolved) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(IkarosError::Message(format!(
                        "self-modify rejects symlink target: {}",
                        resolved.display()
                    )));
                }
                if !is_leaf && !metadata.is_dir() {
                    return Err(IkarosError::Message(format!(
                        "self-modify target parent is not a directory: {}",
                        resolved.display()
                    )));
                }
                let canonical = fs::canonicalize(&resolved)
                    .map_err(|source| IkarosError::io(&resolved, source))?;
                if !canonical.starts_with(workspace_root) {
                    return Err(IkarosError::Message(
                        "self-modify target must stay inside the workspace".into(),
                    ));
                }
            }
            Err(source) if source.kind() == ErrorKind::NotFound => {}
            Err(source) => return Err(IkarosError::io(&resolved, source)),
        }
    }
    Ok(resolved)
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

pub(super) fn diff_file_paths(diff: &str) -> Vec<PathBuf> {
    diff.lines()
        .filter_map(|line| line.strip_prefix("+++ "))
        .filter_map(|path| parse_diff_path(path).ok())
        .collect()
}

pub(super) fn diff_matches_single_target(
    changed_files: &[PathBuf],
    target_path: &Path,
    workspace_root: &Path,
) -> bool {
    let relative_target = workspace_relative_path(target_path, workspace_root);
    changed_files.len() == 1
        && changed_files
            .iter()
            .any(|path| normalized_components_match(path, &relative_target))
}

pub(super) fn workspace_relative_path(path: &Path, workspace_root: &Path) -> PathBuf {
    if let Ok(canonical_root) = fs::canonicalize(workspace_root)
        && let Ok(relative) = path.strip_prefix(&canonical_root)
    {
        return relative.to_path_buf();
    }
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_path_buf()
}

fn normalized_components_match(left: &Path, right: &Path) -> bool {
    normalized_components(left) == normalized_components(right)
}

fn normalized_components(path: &Path) -> Option<Vec<String>> {
    path.components()
        .map(|component| match component {
            Component::CurDir => Some(String::new()),
            Component::Normal(name) => {
                let value = name.to_string_lossy();
                if cfg!(windows) {
                    Some(value.to_ascii_lowercase())
                } else {
                    Some(value.into_owned())
                }
            }
            _ => None,
        })
        .filter(|component| component.as_deref() != Some(""))
        .collect()
}
