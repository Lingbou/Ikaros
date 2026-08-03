use crate::{
    agent::Agent,
    config::{AppConfig, AppPaths},
    provider::OpenAiProvider,
    store::SessionStore,
    tools::ToolBox,
    tui,
};
use anyhow::{Context, Result, bail};
use clap::Parser;
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(name = "ikaros", version, about = "A small terminal-first local agent")]
struct Cli {
    #[arg(long, value_name = "DIR")]
    home: Option<PathBuf>,
    #[arg(long, value_name = "DIR", default_value = ".")]
    workspace: PathBuf,
}

pub(crate) async fn run() -> Result<()> {
    let cli = Cli::parse();
    tokio::select! {
        result = run_cli(cli) => result,
        signal = shutdown_signal() => {
            signal?;
            bail!("interrupted by shutdown signal")
        }
    }
}

async fn run_cli(cli: Cli) -> Result<()> {
    let paths = AppPaths::discover(cli.home)?;
    let _app_lock = paths.acquire_lock()?;
    let workspace = resolve_workspace(&cli.workspace)?;
    let (agent, model) = build_agent(&paths, &workspace)?;
    tui::run(&agent, &workspace, &model).await
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .context("failed to install SIGTERM handler")?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                result.context("failed to install Ctrl-C handler")?;
            }
            _ = terminate.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to install Ctrl-C handler")?;
    }
    Ok(())
}

fn build_agent(paths: &AppPaths, workspace: &Path) -> Result<(Agent<OpenAiProvider>, String)> {
    let config = AppConfig::load(paths)?;
    let provider_config = config.resolve_provider()?;
    let model = provider_config.model.clone();
    let provider = OpenAiProvider::new(
        &provider_config.base_url,
        provider_config.model,
        provider_config.api_key,
    )?;
    let store = SessionStore::open(&paths.database)?;
    let tools = ToolBox::new(workspace)?;
    Ok((Agent::new(provider, store, tools), model))
}

fn resolve_workspace(path: &Path) -> Result<PathBuf> {
    let workspace = path
        .canonicalize()
        .with_context(|| format!("failed to resolve workspace {}", path.display()))?;
    if !workspace.is_dir() {
        bail!("workspace is not a directory: {}", workspace.display());
    }
    Ok(workspace)
}
