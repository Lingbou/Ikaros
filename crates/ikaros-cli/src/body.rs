// SPDX-License-Identifier: GPL-3.0-only

mod dashboard;
mod server;

use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_body::{BodyAdapter, CliBodyAdapter};
use ikaros_core::IkarosPaths;
use ikaros_runtime::base_body_status;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub(crate) enum BodyCommand {
    Status,
    Dashboard(BodyDashboard),
    Serve(BodyServe),
}

#[derive(Debug, Args)]
pub(crate) struct BodyDashboard {
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long = "snapshot-output")]
    snapshot_output: Option<PathBuf>,
    #[arg(long = "refresh-seconds")]
    refresh_seconds: Option<u64>,
    #[arg(long, default_value_t = 12)]
    events: usize,
}

#[derive(Debug, Args)]
pub(crate) struct BodyServe {
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8001)]
    port: u16,
    #[arg(long = "refresh-seconds", default_value_t = 5)]
    refresh_seconds: u64,
    #[arg(long, default_value_t = 12)]
    events: usize,
    #[arg(long, hide = true)]
    once: bool,
}

pub(crate) fn body_command(
    command: BodyCommand,
    paths: &IkarosPaths,
    workspace: &Path,
) -> Result<()> {
    match command {
        BodyCommand::Status => {
            paths.ensure()?;
            let status = base_body_status(paths)?;
            println!("{}", CliBodyAdapter.render_status(&status));
            println!("workspace: {}", workspace.display());
        }
        BodyCommand::Dashboard(args) => dashboard::write_dashboard(args, paths, workspace)?,
        BodyCommand::Serve(args) => server::serve_body_dashboard(args, paths, workspace)?,
    }
    Ok(())
}

#[cfg(test)]
mod tests;
