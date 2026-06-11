// SPDX-License-Identifier: GPL-3.0-only

use super::super::issue::PluginLoadIssue;
use ikaros_core::RiskLevel;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginMarketplaceUpdate {
    pub name: String,
    pub enabled: bool,
    pub marketplace_path: PathBuf,
    pub entry_added: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginInstallReport {
    pub name: String,
    pub version: String,
    pub source_path: PathBuf,
    pub target_dir: PathBuf,
    pub enabled: bool,
    pub replaced: bool,
    pub skill_count: usize,
    pub command_skill_count: usize,
    pub marketplace_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginUninstallReport {
    pub name: String,
    pub manifest_path: PathBuf,
    pub removed_path: PathBuf,
    pub marketplace_path: PathBuf,
    pub marketplace_entry_removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginValidationReport {
    pub path: PathBuf,
    pub name: String,
    pub version: String,
    pub skill_count: usize,
    pub command_skill_count: usize,
    pub risk_levels: Vec<RiskLevel>,
    pub missing_command_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginAuditReport {
    pub plugin_count: usize,
    pub enabled_plugin_count: usize,
    pub disabled_plugin_count: usize,
    pub skill_count: usize,
    pub enabled_skill_count: usize,
    pub command_skill_count: usize,
    pub warning_count: usize,
    pub missing_command_count: usize,
    pub plugins: Vec<PluginAuditPlugin>,
    pub warnings: Vec<PluginLoadIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginAuditPlugin {
    pub name: String,
    pub version: String,
    pub enabled: bool,
    pub priority: i32,
    pub source: String,
    pub marketplace_path: Option<PathBuf>,
    pub manifest_path: PathBuf,
    pub skill_count: usize,
    pub enabled_skill_count: usize,
    pub command_skill_count: usize,
    pub risk_levels: Vec<RiskLevel>,
    pub missing_commands: Vec<PluginAuditMissingCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PluginAuditMissingCommand {
    pub skill_name: String,
    pub program: PathBuf,
    pub resolved_path: PathBuf,
}
