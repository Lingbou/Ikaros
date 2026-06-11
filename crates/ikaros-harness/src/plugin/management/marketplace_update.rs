// SPDX-License-Identifier: GPL-3.0-only

use super::super::{
    catalog::PluginCatalog,
    loader::{load_plugin_marketplace_entries, marketplace_path},
    marketplace::{PluginMarketplace, PluginMarketplaceEntry},
    validation::validate_plugin_marketplace,
};
use super::types::PluginMarketplaceUpdate;
use ikaros_core::{IkarosError, Result};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub fn set_plugin_enabled(
    skills_dir: impl AsRef<Path>,
    name: &str,
    enabled: bool,
) -> Result<PluginMarketplaceUpdate> {
    let skills_dir = skills_dir.as_ref();
    fs::create_dir_all(skills_dir).map_err(|source| IkarosError::io(skills_dir, source))?;

    let catalog = PluginCatalog::load(skills_dir)?;
    let plugin = catalog
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.name == name)
        .ok_or_else(|| IkarosError::Message(format!("plugin not found: {name}")))?;
    let inferred_path = infer_marketplace_path(skills_dir, &plugin.path);

    let mut marketplace = load_plugin_marketplace_entries(skills_dir).map_err(|issue| {
        IkarosError::Message(format!(
            "failed to load plugin marketplace at {}: {}",
            issue.path.display(),
            issue.message
        ))
    })?;
    let mut entry_added = false;
    if let Some(entry) = marketplace
        .plugins
        .iter_mut()
        .find(|entry| entry.name == name)
    {
        entry.enabled = enabled;
        if entry.path.is_none() {
            entry.path = inferred_path.clone();
        }
    } else {
        let mut entry = PluginMarketplaceEntry::local_default(name);
        entry.enabled = enabled;
        entry.path = inferred_path;
        marketplace.plugins.push(entry);
        entry_added = true;
    }

    write_marketplace(skills_dir, marketplace)?;

    Ok(PluginMarketplaceUpdate {
        name: name.to_owned(),
        enabled,
        marketplace_path: marketplace_path(skills_dir),
        entry_added,
    })
}

pub(super) fn write_marketplace(
    skills_dir: &Path,
    mut marketplace: PluginMarketplace,
) -> Result<()> {
    marketplace.plugins.sort_by(|left, right| {
        left.priority
            .cmp(&right.priority)
            .then(left.name.cmp(&right.name))
    });
    validate_plugin_marketplace(marketplace.clone())?;
    let path = marketplace_path(skills_dir);
    let encoded = toml::to_string_pretty(&marketplace)
        .map_err(|source| IkarosError::Message(format!("toml serialize error: {source}")))?;
    fs::write(&path, encoded).map_err(|source| IkarosError::io(&path, source))?;
    Ok(())
}

fn infer_marketplace_path(skills_dir: &Path, manifest_path: &Path) -> Option<PathBuf> {
    let relative = manifest_path.strip_prefix(skills_dir).ok()?;
    if relative
        .file_name()
        .is_some_and(|name| name == "plugin.toml")
    {
        return relative.parent().and_then(|parent| {
            if parent.as_os_str().is_empty() {
                None
            } else {
                Some(parent.to_path_buf())
            }
        });
    }
    Some(relative.to_path_buf())
}
