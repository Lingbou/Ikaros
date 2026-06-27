// SPDX-License-Identifier: GPL-3.0-only

mod catalog;
mod issue;
mod loader;
mod management;
mod manifest;
mod marketplace;
mod validation;

pub use catalog::{LoadedPluginManifest, PluginCatalog};
pub use issue::PluginLoadIssue;
pub use management::{
    PluginAuditMissingCommand, PluginAuditPlugin, PluginAuditReport, PluginInstallReport,
    PluginMarketplaceUpdate, PluginUninstallReport, PluginValidationReport, audit_plugins,
    install_local_plugin, set_plugin_enabled, set_plugin_quarantine, uninstall_local_plugin,
    validate_plugin_file,
};
pub use manifest::{
    PLUGIN_COMMAND_MAX_ARG_BYTES, PLUGIN_COMMAND_MAX_ARGS, PLUGIN_COMMAND_MAX_OUTPUT_BYTES,
    PLUGIN_COMMAND_MAX_STDIN_BYTES, PLUGIN_COMMAND_MAX_TIMEOUT_MS, PluginCommandManifest,
    PluginManifest, PluginPermissionDeclaration, PluginSkillManifest,
};
pub use marketplace::{PluginMarketplace, PluginMarketplaceEntry};

#[cfg(test)]
mod tests;
