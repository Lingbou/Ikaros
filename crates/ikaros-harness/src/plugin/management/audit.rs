// SPDX-License-Identifier: GPL-3.0-only

use super::super::catalog::PluginCatalog;
use super::types::{PluginAuditMissingCommand, PluginAuditPlugin, PluginAuditReport};
use ikaros_core::Result;
use std::path::Path;

pub fn audit_plugins(skills_dir: impl AsRef<Path>) -> Result<PluginAuditReport> {
    let catalog = PluginCatalog::load(skills_dir)?;
    let mut plugins = Vec::new();
    let mut skill_count = 0;
    let mut enabled_skill_count = 0;
    let mut command_skill_count = 0;
    let mut missing_command_count = 0;

    for plugin in &catalog.plugins {
        let mut plugin_command_skill_count = 0;
        let mut risk_levels = Vec::new();
        let mut missing_commands = Vec::new();
        let manifest_dir = plugin.path.parent().unwrap_or_else(|| Path::new("."));

        for skill in &plugin.manifest.skills {
            if !risk_levels.contains(&skill.risk) {
                risk_levels.push(skill.risk.clone());
            }
            if let Some(command) = &skill.command {
                plugin_command_skill_count += 1;
                let resolved_path = manifest_dir.join(&command.program);
                if !resolved_path.exists() {
                    missing_commands.push(PluginAuditMissingCommand {
                        skill_name: format!("{}.{}", plugin.manifest.name, skill.name),
                        program: command.program.clone(),
                        resolved_path,
                    });
                }
            }
        }

        let plugin_skill_count = plugin.manifest.skills.len();
        let plugin_enabled_skill_count = if plugin.marketplace.enabled {
            plugin_skill_count
        } else {
            0
        };
        skill_count += plugin_skill_count;
        enabled_skill_count += plugin_enabled_skill_count;
        command_skill_count += plugin_command_skill_count;
        missing_command_count += missing_commands.len();

        plugins.push(PluginAuditPlugin {
            name: plugin.manifest.name.clone(),
            version: plugin.manifest.version.clone(),
            enabled: plugin.marketplace.enabled,
            priority: plugin.marketplace.priority,
            source: plugin.marketplace.source.clone(),
            marketplace_path: plugin.marketplace.path.clone(),
            manifest_path: plugin.path.clone(),
            skill_count: plugin_skill_count,
            enabled_skill_count: plugin_enabled_skill_count,
            command_skill_count: plugin_command_skill_count,
            risk_levels,
            missing_commands,
        });
    }

    Ok(PluginAuditReport {
        plugin_count: catalog.plugin_count(),
        enabled_plugin_count: catalog.enabled_plugin_count(),
        disabled_plugin_count: catalog.disabled_plugin_count(),
        skill_count,
        enabled_skill_count,
        command_skill_count,
        warning_count: catalog.warnings.len(),
        missing_command_count,
        plugins,
        warnings: catalog.warnings,
    })
}
