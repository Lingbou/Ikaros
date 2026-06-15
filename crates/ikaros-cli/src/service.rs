// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result};
use clap::{Args, Subcommand, ValueEnum};
use ikaros_core::IkarosPaths;
use ikaros_service::{ServiceKind, ServiceManager, ServiceTemplateConfig};
use std::{
    fs,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Subcommand)]
pub(crate) enum ServiceCommand {
    Render(ServiceRender),
}

#[derive(Debug, Args)]
pub(crate) struct ServiceRender {
    #[arg(long, value_enum)]
    kind: ServiceKindArg,
    #[arg(long, value_enum, default_value = "systemd")]
    manager: ServiceManagerArg,
    #[arg(long)]
    output: Option<PathBuf>,
    #[arg(long = "bin")]
    binary_path: Option<PathBuf>,
    #[arg(long)]
    label: Option<String>,
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8002)]
    port: u16,
    #[arg(long = "interval-seconds", default_value_t = 60)]
    interval_seconds: u64,
    #[arg(long, default_value_t = 10)]
    limit: usize,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ServiceKindArg {
    ScheduleWorker,
    MessageWorker,
    MessageWebhook,
}

impl From<ServiceKindArg> for ServiceKind {
    fn from(value: ServiceKindArg) -> Self {
        match value {
            ServiceKindArg::ScheduleWorker => Self::ScheduleWorker,
            ServiceKindArg::MessageWorker => Self::MessageWorker,
            ServiceKindArg::MessageWebhook => Self::MessageWebhook,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ServiceManagerArg {
    Systemd,
    Launchd,
}

impl From<ServiceManagerArg> for ServiceManager {
    fn from(value: ServiceManagerArg) -> Self {
        match value {
            ServiceManagerArg::Systemd => Self::Systemd,
            ServiceManagerArg::Launchd => Self::Launchd,
        }
    }
}

pub(crate) fn service_command(
    command: ServiceCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        ServiceCommand::Render(args) => render_service(args, paths, workspace, agent_override)?,
    }
    Ok(())
}

fn render_service(
    args: ServiceRender,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    if args.interval_seconds == 0 {
        anyhow::bail!("service interval must be greater than zero");
    }
    if args.limit == 0 {
        anyhow::bail!("service limit must be greater than zero");
    }
    let kind = ServiceKind::from(args.kind);
    let binary_path = match args.binary_path {
        Some(path) => path,
        None => std::env::current_exe().with_context(|| "failed to resolve current binary path")?,
    };
    let label = args
        .label
        .unwrap_or_else(|| ServiceTemplateConfig::default_label(kind).into());
    let config = ServiceTemplateConfig {
        manager: args.manager.into(),
        kind,
        label,
        binary_path,
        ikaros_home: paths.home.clone(),
        workspace: workspace.to_path_buf(),
        agent: agent_override.map(ToOwned::to_owned),
        host: args.host,
        port: args.port,
        interval_seconds: args.interval_seconds,
        limit: args.limit,
    };
    let rendered = config.render();
    if let Some(output) = args.output {
        let output = output_path_under_home(paths, output)?;
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&output, &rendered)
            .with_context(|| format!("failed to write {}", output.display()))?;
        println!("service_template: {}", output.display());
    } else {
        print!("{rendered}");
    }
    Ok(())
}

fn output_path_under_home(paths: &IkarosPaths, output: PathBuf) -> Result<PathBuf> {
    if output
        .components()
        .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        anyhow::bail!("service template output must be relative to IKAROS_HOME");
    }
    if output
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        anyhow::bail!("service template output cannot contain '..'");
    }
    if output
        .components()
        .any(|component| component.as_os_str() == ".temp")
    {
        anyhow::bail!("service template output cannot target .temp");
    }
    Ok(paths.home.join(output))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_output_path_stays_under_home() {
        let paths = IkarosPaths::from_home("/tmp/ikaros-home");
        assert_eq!(
            output_path_under_home(&paths, PathBuf::from("services/worker.service")).expect("path"),
            PathBuf::from("/tmp/ikaros-home/services/worker.service")
        );
        assert!(output_path_under_home(&paths, PathBuf::from("../worker.service")).is_err());
        assert!(output_path_under_home(&paths, PathBuf::from("/tmp/worker.service")).is_err());
        assert!(output_path_under_home(&paths, PathBuf::from(".temp/worker.service")).is_err());
    }
}
