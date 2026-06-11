// SPDX-License-Identifier: GPL-3.0-only

mod audit;
mod fs_ops;
mod install;
mod marketplace_update;
mod types;
mod uninstall;
mod validation;

pub use audit::audit_plugins;
pub use install::install_local_plugin;
pub use marketplace_update::set_plugin_enabled;
pub use types::{
    PluginAuditMissingCommand, PluginAuditPlugin, PluginAuditReport, PluginInstallReport,
    PluginMarketplaceUpdate, PluginUninstallReport, PluginValidationReport,
};
pub use uninstall::uninstall_local_plugin;
pub use validation::validate_plugin_file;
