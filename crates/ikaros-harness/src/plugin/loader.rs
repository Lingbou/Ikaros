// SPDX-License-Identifier: GPL-3.0-only

use super::{
    issue::PluginLoadIssue,
    manifest::PluginManifest,
    marketplace::{PluginMarketplace, PluginMarketplaceEntry},
    validation::{redact_plugin_manifest, validate_plugin_manifest, validate_plugin_marketplace},
};
use ikaros_core::{IkarosError, Result, redact_secrets};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

pub(super) fn discover_plugin_manifests(skills_dir: &Path) -> Result<Vec<PathBuf>> {
    let mut manifests = Vec::new();
    for entry in fs::read_dir(skills_dir).map_err(|source| IkarosError::io(skills_dir, source))? {
        let entry = entry.map_err(|source| IkarosError::io(skills_dir, source))?;
        let path = entry.path();
        if path.is_dir() {
            let nested = path.join("plugin.toml");
            if nested.exists() {
                manifests.push(nested);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == "marketplace.toml")
        {
            continue;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("toml"))
        {
            manifests.push(path);
        }
    }
    manifests.sort();
    Ok(manifests)
}

pub(super) fn load_plugin_marketplace(
    skills_dir: &Path,
) -> std::result::Result<BTreeMap<String, PluginMarketplaceEntry>, PluginLoadIssue> {
    let marketplace = load_plugin_marketplace_entries(skills_dir)?;
    validate_plugin_marketplace(marketplace).map_err(|source| PluginLoadIssue {
        path: marketplace_path(skills_dir),
        message: redact_secrets(&source.to_string()),
    })
}

pub(super) fn load_plugin_marketplace_entries(
    skills_dir: &Path,
) -> std::result::Result<PluginMarketplace, PluginLoadIssue> {
    let path = marketplace_path(skills_dir);
    if !path.exists() {
        return Ok(PluginMarketplace::default());
    }
    let raw = fs::read_to_string(&path).map_err(|source| PluginLoadIssue {
        path: path.clone(),
        message: redact_secrets(&source.to_string()),
    })?;
    let marketplace =
        toml::from_str::<PluginMarketplace>(&raw).map_err(|source| PluginLoadIssue {
            path: path.clone(),
            message: redact_secrets(&source.to_string()),
        })?;
    Ok(marketplace)
}

pub fn marketplace_path(skills_dir: &Path) -> PathBuf {
    skills_dir.join("marketplace.toml")
}

pub(super) fn load_plugin_manifest(path: &Path) -> Result<PluginManifest> {
    let raw = fs::read_to_string(path).map_err(|source| IkarosError::io(path, source))?;
    let manifest = toml::from_str::<PluginManifest>(&raw).map_err(|source| {
        IkarosError::Message(format!(
            "failed to parse plugin manifest at {}: {}",
            path.display(),
            redact_secrets(&source.to_string())
        ))
    })?;
    validate_plugin_manifest(&manifest)?;
    Ok(redact_plugin_manifest(manifest))
}
