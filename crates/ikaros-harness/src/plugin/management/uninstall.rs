// SPDX-License-Identifier: GPL-3.0-only

use super::super::{
    catalog::PluginCatalog,
    loader::{load_plugin_marketplace_entries, marketplace_path},
};
use super::{
    fs_ops::{removal_target_for_manifest, remove_existing_target},
    marketplace_update::write_marketplace,
    types::PluginUninstallReport,
};
use ikaros_core::{IkarosError, Result};
use std::path::Path;

pub fn uninstall_local_plugin(
    skills_dir: impl AsRef<Path>,
    name: &str,
) -> Result<PluginUninstallReport> {
    let skills_dir = skills_dir.as_ref();
    let catalog = PluginCatalog::load(skills_dir)?;
    let plugin = catalog
        .plugins
        .iter()
        .find(|plugin| plugin.manifest.name == name)
        .ok_or_else(|| IkarosError::Message(format!("plugin not found: {name}")))?;
    let manifest_path = plugin.path.clone();
    let removed_path = removal_target_for_manifest(skills_dir, &manifest_path)?;
    remove_existing_target(&removed_path)?;

    let mut marketplace = load_plugin_marketplace_entries(skills_dir).map_err(|issue| {
        IkarosError::Message(format!(
            "failed to load plugin marketplace at {}: {}",
            issue.path.display(),
            issue.message
        ))
    })?;
    let before = marketplace.plugins.len();
    marketplace.plugins.retain(|entry| entry.name != name);
    let marketplace_entry_removed = before != marketplace.plugins.len();
    write_marketplace(skills_dir, marketplace)?;

    Ok(PluginUninstallReport {
        name: name.to_owned(),
        manifest_path,
        removed_path,
        marketplace_path: marketplace_path(skills_dir),
        marketplace_entry_removed,
    })
}
