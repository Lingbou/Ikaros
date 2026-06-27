// SPDX-License-Identifier: GPL-3.0-only

use crate::session_and_registry;
use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_core::IkarosPaths;
use std::path::{Path, PathBuf};

const DEFAULT_CDP_ENDPOINT: &str = "http://127.0.0.1:9222";
mod cdp;
mod http;
mod supervisor;
#[cfg(test)]
mod tests;
mod workbench;

use cdp::browser_cdp_cli_output;
use http::{
    browser_response_json, cdp_endpoint_url, cdp_new_target_path, cdp_target_path, send_cdp_request,
};
use supervisor::browser_supervisor_cli_output;
pub(crate) use workbench::run_browser_workbench_command;

#[derive(Debug, Subcommand)]
pub(crate) enum BrowserCommand {
    Launch(BrowserLaunchArgs),
    SupervisorStatus(BrowserSupervisorStatusArgs),
    Stop(BrowserStopArgs),
    Status(BrowserCdpArgs),
    List(BrowserCdpArgs),
    New(BrowserNewArgs),
    Activate(BrowserTargetArgs),
    Close(BrowserTargetArgs),
    Navigate(BrowserNavigateArgs),
    Snapshot(BrowserTargetArgs),
    Click(BrowserClickArgs),
    Type(BrowserTypeArgs),
    Scroll(BrowserScrollArgs),
    Screenshot(BrowserScreenshotArgs),
    Cdp(BrowserCdpCommandArgs),
}

#[derive(Debug, Args)]
pub(crate) struct BrowserLaunchArgs {
    #[arg(long)]
    browser: Option<PathBuf>,
    #[arg(long = "remote-debugging-port", default_value_t = 9222)]
    remote_debugging_port: u16,
    #[arg(long = "user-data-dir")]
    user_data_dir: Option<PathBuf>,
    #[arg(long)]
    headless: bool,
    #[arg(long = "profile", default_value = "default")]
    profile: String,
    #[arg(long)]
    url: Option<String>,
    #[arg(last = true)]
    extra_args: Vec<String>,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserSupervisorStatusArgs {
    #[arg(long = "profile", default_value = "default")]
    profile: String,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserStopArgs {
    #[arg(long = "profile", default_value = "default")]
    profile: String,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserCdpArgs {
    #[arg(long, default_value = DEFAULT_CDP_ENDPOINT)]
    endpoint: String,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserNewArgs {
    #[command(flatten)]
    cdp: BrowserCdpArgs,
    #[arg(default_value = "about:blank")]
    url: String,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserTargetArgs {
    #[command(flatten)]
    cdp: BrowserCdpArgs,
    target_id: String,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserNavigateArgs {
    #[command(flatten)]
    cdp: BrowserCdpArgs,
    target_id: String,
    url: String,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserClickArgs {
    #[command(flatten)]
    cdp: BrowserCdpArgs,
    target_id: String,
    x: f64,
    y: f64,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserTypeArgs {
    #[command(flatten)]
    cdp: BrowserCdpArgs,
    target_id: String,
    text: String,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserScrollArgs {
    #[command(flatten)]
    cdp: BrowserCdpArgs,
    target_id: String,
    #[arg(long, default_value_t = 0.0)]
    x: f64,
    #[arg(long, default_value_t = 600.0)]
    y: f64,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserScreenshotArgs {
    #[command(flatten)]
    cdp: BrowserCdpArgs,
    target_id: String,
    #[arg(long, default_value = "png")]
    format: String,
}

#[derive(Debug, Args)]
pub(crate) struct BrowserCdpCommandArgs {
    #[command(flatten)]
    cdp: BrowserCdpArgs,
    target_id: String,
    method: String,
    #[arg(long, default_value = "{}")]
    params_json: String,
}

pub(crate) async fn browser_command(
    command: BrowserCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    if let Some(output) = browser_supervisor_cli_output(paths, &command)? {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    let (session, _) = session_and_registry(paths, workspace, agent_override)?;
    if let Some(output) = browser_cdp_cli_output(&session, &command).await? {
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }
    let (schema, action, method, path, endpoint, target_url_policy): (
        &str,
        &str,
        &str,
        String,
        &str,
        Option<&str>,
    ) = match &command {
        BrowserCommand::Launch(_)
        | BrowserCommand::SupervisorStatus(_)
        | BrowserCommand::Stop(_) => unreachable!("browser supervisor command handled above"),
        BrowserCommand::Status(args) => (
            "ikaros-browser-cdp-status-v1",
            "status",
            "GET",
            "/json/version".into(),
            args.endpoint.as_str(),
            None,
        ),
        BrowserCommand::List(args) => (
            "ikaros-browser-cdp-list-v1",
            "list",
            "GET",
            "/json/list".into(),
            args.endpoint.as_str(),
            None,
        ),
        BrowserCommand::New(args) => (
            "ikaros-browser-cdp-new-target-v1",
            "new",
            "PUT",
            cdp_new_target_path(&args.url)?,
            args.cdp.endpoint.as_str(),
            Some("cdp_endpoint_governed_target_page_loaded_by_browser"),
        ),
        BrowserCommand::Activate(args) => (
            "ikaros-browser-cdp-activate-target-v1",
            "activate",
            "GET",
            cdp_target_path("/json/activate", &args.target_id)?,
            args.cdp.endpoint.as_str(),
            None,
        ),
        BrowserCommand::Close(args) => (
            "ikaros-browser-cdp-close-target-v1",
            "close",
            "GET",
            cdp_target_path("/json/close", &args.target_id)?,
            args.cdp.endpoint.as_str(),
            None,
        ),
        BrowserCommand::Navigate(_)
        | BrowserCommand::Snapshot(_)
        | BrowserCommand::Click(_)
        | BrowserCommand::Type(_)
        | BrowserCommand::Scroll(_)
        | BrowserCommand::Screenshot(_)
        | BrowserCommand::Cdp(_) => unreachable!("CDP websocket command handled above"),
    };
    let url = cdp_endpoint_url(endpoint, &path);
    let response = send_cdp_request(&session, method, &url).await?;
    let output = browser_response_json(
        schema,
        action,
        endpoint,
        &url,
        target_url_policy,
        &response,
        session.audit.path().display().to_string(),
    );
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}
