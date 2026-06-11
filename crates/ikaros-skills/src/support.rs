// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use std::path::{Path, PathBuf};

pub(crate) fn input_path(input: &serde_json::Value, workspace_root: &Path) -> Result<PathBuf> {
    let raw = input_string(input, "path")?;
    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(workspace_root.join(path))
    }
}

pub(crate) fn optional_input_path(
    input: &serde_json::Value,
    key: &str,
    workspace_root: &Path,
) -> Option<PathBuf> {
    input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(|raw| {
            let path = PathBuf::from(raw);
            if path.is_absolute() {
                path
            } else {
                workspace_root.join(path)
            }
        })
}

pub(crate) fn input_string(input: &serde_json::Value, key: &str) -> Result<String> {
    input
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| IkarosError::Message(format!("{key} is required")))
}
