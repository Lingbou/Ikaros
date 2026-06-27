// SPDX-License-Identifier: GPL-3.0-only

use super::super::loader::load_plugin_manifest;
use super::{fs_ops::resolve_manifest_path, types::PluginValidationReport};
use ikaros_core::Result;
use std::path::Path;

pub fn validate_plugin_file(path: impl AsRef<Path>) -> Result<PluginValidationReport> {
    let path = resolve_manifest_path(path.as_ref());
    let manifest = load_plugin_manifest(&path)?;
    let plugin_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut risk_levels = Vec::new();
    let mut command_skill_count = 0;
    let mut missing_command_paths = Vec::new();

    for skill in &manifest.skills {
        if !risk_levels.contains(&skill.risk) {
            risk_levels.push(skill.risk.clone());
        }
        if let Some(command) = &skill.command {
            command_skill_count += 1;
            let command_path = plugin_dir.join(&command.program);
            if !command_path.exists() {
                missing_command_paths.push(command.program.clone());
            }
        }
    }

    Ok(PluginValidationReport {
        path,
        name: manifest.name,
        version: manifest.version,
        skill_count: manifest.skills.len(),
        command_skill_count,
        risk_levels,
        missing_command_paths,
    })
}
