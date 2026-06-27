// SPDX-License-Identifier: GPL-3.0-only

use super::types::SandboxProfile;
use std::{
    ffi::OsString,
    fs,
    path::{Component, Path, PathBuf},
};

pub(crate) fn resolve_under_workspace(path: &Path, workspace_root: &Path) -> PathBuf {
    let workspace_root = canonicalize_path_for_policy(workspace_root);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        workspace_root.join(path)
    };
    canonicalize_path_for_policy(&candidate)
}

pub(crate) fn canonicalize_path_for_policy(path: &Path) -> PathBuf {
    let normalized = normalize_path(path);
    if let Ok(canonical) = fs::canonicalize(&normalized) {
        return normalize_path(&canonical);
    }

    let mut missing = Vec::<OsString>::new();
    let mut current = normalized.as_path();
    loop {
        if let Some(name) = current.file_name() {
            missing.push(name.to_os_string());
        }

        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current {
            break;
        }

        if let Ok(canonical_parent) = fs::canonicalize(parent) {
            let mut rebuilt = canonical_parent;
            for component in missing.iter().rev() {
                rebuilt.push(component);
            }
            return normalize_path(&rebuilt);
        }
        current = parent;
    }

    normalized
}

pub(crate) fn normalize_path(path: &Path) -> PathBuf {
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

pub(super) fn is_under(path: &Path, root: &Path) -> bool {
    let root = canonicalize_path_for_policy(root);
    let path = canonicalize_path_for_policy(path);
    path.starts_with(root)
}

pub(super) fn is_protected(path: &Path, sandbox: &SandboxProfile) -> bool {
    sandbox.protected_paths.iter().any(|protected| {
        let protected = resolve_under_workspace(protected, &sandbox.workspace_root);
        path.starts_with(protected)
    })
}

pub(super) fn has_component(path: &Path, name: &str) -> bool {
    path.components()
        .any(|component| component.as_os_str() == name)
}

pub(super) fn is_secret_like_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| {
            matches!(
                name,
                ".env" | ".env.local" | ".env.production" | "id_rsa" | "id_ed25519"
            ) || name.contains("secret")
                || name.contains("token")
                || name.contains("password")
        })
}
