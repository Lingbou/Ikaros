// SPDX-License-Identifier: GPL-3.0-only

use super::super::loader::load_plugin_manifest;
use super::{
    fs_ops::{
        copy_plugin_dir, copy_regular_file, ensure_install_target_path, reject_self_replacement,
        reject_temp_path, remove_existing_target, resolve_manifest_path,
    },
    marketplace_update::set_plugin_enabled,
    types::PluginInstallReport,
    validation::validate_plugin_file,
};
use ikaros_core::{IkarosError, Result};
use std::{fs, path::Path};

pub fn install_local_plugin(
    skills_dir: impl AsRef<Path>,
    source_path: impl AsRef<Path>,
    enabled: bool,
    force: bool,
) -> Result<PluginInstallReport> {
    let skills_dir = skills_dir.as_ref();
    let source_path = source_path.as_ref();
    reject_temp_path(source_path, "plugin source path")?;
    fs::create_dir_all(skills_dir).map_err(|source| IkarosError::io(skills_dir, source))?;

    let manifest_path = resolve_manifest_path(source_path);
    let manifest = load_plugin_manifest(&manifest_path)?;
    let validation = validate_plugin_file(source_path)?;
    if !validation.missing_command_paths.is_empty() {
        return Err(IkarosError::Message(format!(
            "plugin {} has missing command path(s): {}",
            manifest.name,
            validation
                .missing_command_paths
                .iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }

    let target_dir = skills_dir.join(&manifest.name);
    ensure_install_target_path(skills_dir, &target_dir)?;
    let replaced = target_dir.exists();
    if replaced {
        if !force {
            return Err(IkarosError::Message(format!(
                "plugin already installed: {}; pass --force to replace",
                manifest.name
            )));
        }
        reject_self_replacement(source_path, &target_dir)?;
        remove_existing_target(&target_dir)?;
    }

    if source_path.is_dir() {
        copy_plugin_dir(source_path, &target_dir)?;
    } else {
        fs::create_dir_all(&target_dir).map_err(|source| IkarosError::io(&target_dir, source))?;
        copy_regular_file(source_path, &target_dir.join("plugin.toml"))?;
    }

    let marketplace_update = set_plugin_enabled(skills_dir, &manifest.name, enabled)?;

    Ok(PluginInstallReport {
        name: manifest.name,
        version: manifest.version,
        source_path: source_path.to_path_buf(),
        target_dir,
        enabled: marketplace_update.enabled,
        replaced,
        skill_count: validation.skill_count,
        command_skill_count: validation.command_skill_count,
        marketplace_path: marketplace_update.marketplace_path,
    })
}
