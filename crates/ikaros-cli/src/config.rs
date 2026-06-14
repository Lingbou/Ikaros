// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, bail};
use clap::Subcommand;
use ikaros_core::{ConfigValidationReport, IkarosConfig, IkarosPaths};

#[derive(Debug, Subcommand)]
pub(crate) enum ConfigCommand {
    /// Validate IKAROS_HOME/config.yaml without printing secret values.
    Validate,
}

pub(crate) fn config_command(command: ConfigCommand, paths: &IkarosPaths) -> Result<()> {
    match command {
        ConfigCommand::Validate => validate_config(paths),
    }
}

fn validate_config(paths: &IkarosPaths) -> Result<()> {
    let report = IkarosConfig::validate_file(&paths.config)?;
    print_report(&report);
    if report.is_valid() {
        println!("config valid: {}", paths.config.display());
        Ok(())
    } else {
        bail!(
            "configuration validation failed: {}",
            paths.config.display()
        )
    }
}

fn print_report(report: &ConfigValidationReport) {
    for warning in &report.warnings {
        println!("warning: {}: {}", warning.path, warning.message);
    }
    for error in &report.errors {
        println!("error: {}: {}", error.path, error.message);
    }
}
