// SPDX-License-Identifier: GPL-3.0-only

use crate::session_and_registry;
use anyhow::{Context, Result, bail};
use base64::Engine;
use clap::{Args, Subcommand};
use futures_util::{SinkExt, StreamExt};
use ikaros_core::{IkarosPaths, now_rfc3339, redact_json, redact_secrets};
use ikaros_harness::{ExecutionSession, NetworkEgressRequest, NetworkEgressResponse};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::{Url, form_urlencoded::byte_serialize};

const DEFAULT_CDP_ENDPOINT: &str = "http://127.0.0.1:9222";

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

fn browser_supervisor_cli_output(
    paths: &IkarosPaths,
    command: &BrowserCommand,
) -> Result<Option<Value>> {
    match command {
        BrowserCommand::Launch(args) => Ok(Some(launch_browser_supervisor(paths, args)?)),
        BrowserCommand::SupervisorStatus(args) => Ok(Some(browser_supervisor_status(
            paths,
            &args.profile,
            "status",
        )?)),
        BrowserCommand::Stop(args) => Ok(Some(stop_browser_supervisor(paths, &args.profile)?)),
        _ => Ok(None),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BrowserSupervisorState {
    schema: String,
    version: u32,
    profile: String,
    pid: u32,
    endpoint: String,
    remote_debugging_port: u16,
    user_data_dir: PathBuf,
    browser: PathBuf,
    started_at: String,
    log_path: PathBuf,
    headless: bool,
}

fn launch_browser_supervisor(paths: &IkarosPaths, args: &BrowserLaunchArgs) -> Result<Value> {
    let profile = clean_browser_profile(&args.profile);
    let browser = args.browser.clone().unwrap_or_else(default_browser_binary);
    let user_data_dir = args
        .user_data_dir
        .clone()
        .unwrap_or_else(|| browser_profile_dir(paths, &profile));
    fs::create_dir_all(&user_data_dir).with_context(|| {
        format!(
            "failed to create browser profile {}",
            user_data_dir.display()
        )
    })?;
    let supervisor_dir = browser_supervisor_dir(paths);
    fs::create_dir_all(&supervisor_dir).with_context(|| {
        format!(
            "failed to create browser supervisor dir {}",
            supervisor_dir.display()
        )
    })?;
    let log_path = supervisor_dir.join(format!("{profile}.log"));
    let log = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("failed to open browser log {}", log_path.display()))?;
    let mut process = Command::new(&browser);
    process
        .arg(format!(
            "--remote-debugging-port={}",
            args.remote_debugging_port
        ))
        .arg(format!("--user-data-dir={}", user_data_dir.display()))
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg("--disable-background-networking")
        .arg("--disable-sync");
    if args.headless {
        process.arg("--headless=new");
    }
    for extra in &args.extra_args {
        validate_browser_process_arg(extra)?;
        process.arg(extra);
    }
    if let Some(url) = args.url.as_deref() {
        validate_browser_target_url(url)?;
        process.arg(url);
    }
    let child = process
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .spawn()
        .with_context(|| format!("failed to launch browser {}", browser.display()))?;
    let state = BrowserSupervisorState {
        schema: "ikaros-browser-supervisor-v1".into(),
        version: 1,
        profile,
        pid: child.id(),
        endpoint: format!("http://127.0.0.1:{}", args.remote_debugging_port),
        remote_debugging_port: args.remote_debugging_port,
        user_data_dir,
        browser,
        started_at: now_rfc3339()?,
        log_path,
        headless: args.headless,
    };
    write_browser_supervisor_state(paths, &state)?;
    Ok(browser_supervisor_state_json(
        "launch",
        &state,
        browser_pid_is_running(state.pid),
        "started",
    ))
}

fn browser_supervisor_status(paths: &IkarosPaths, profile: &str, action: &str) -> Result<Value> {
    let profile = clean_browser_profile(profile);
    match read_browser_supervisor_state(paths, &profile)? {
        Some(state) => Ok(browser_supervisor_state_json(
            action,
            &state,
            browser_pid_is_running(state.pid),
            "loaded",
        )),
        None => Ok(json!({
            "schema": "ikaros-browser-supervisor-v1",
            "version": 1,
            "action": action,
            "profile": profile,
            "running": false,
            "status": "missing",
            "state_path": browser_supervisor_state_path(paths, &profile),
        })),
    }
}

fn stop_browser_supervisor(paths: &IkarosPaths, profile: &str) -> Result<Value> {
    let profile = clean_browser_profile(profile);
    let Some(state) = read_browser_supervisor_state(paths, &profile)? else {
        return browser_supervisor_status(paths, &profile, "stop");
    };
    let before = browser_pid_is_running(state.pid);
    let signal = stop_browser_process(state.pid);
    let after = browser_pid_is_running(state.pid);
    Ok(browser_supervisor_state_json(
        "stop",
        &state,
        after,
        if before {
            signal.as_deref().unwrap_or("stop-requested")
        } else {
            "already-stopped"
        },
    ))
}

fn browser_supervisor_state_json(
    action: &str,
    state: &BrowserSupervisorState,
    running: bool,
    status: &str,
) -> Value {
    redact_json(json!({
        "schema": "ikaros-browser-supervisor-v1",
        "version": 1,
        "action": action,
        "profile": &state.profile,
        "running": running,
        "status": status,
        "pid": state.pid,
        "endpoint": &state.endpoint,
        "remote_debugging_port": state.remote_debugging_port,
        "user_data_dir": &state.user_data_dir,
        "browser": &state.browser,
        "started_at": &state.started_at,
        "log_path": &state.log_path,
        "headless": state.headless,
        "state_path": browser_supervisor_state_path_for_state(state),
    }))
}

fn write_browser_supervisor_state(
    paths: &IkarosPaths,
    state: &BrowserSupervisorState,
) -> Result<()> {
    let path = browser_supervisor_state_path(paths, &state.profile);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("browser supervisor state path has no parent"))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create browser state dir {}", parent.display()))?;
    fs::write(&path, serde_json::to_string_pretty(state)?)
        .with_context(|| format!("failed to write browser state {}", path.display()))
}

fn read_browser_supervisor_state(
    paths: &IkarosPaths,
    profile: &str,
) -> Result<Option<BrowserSupervisorState>> {
    let path = browser_supervisor_state_path(paths, profile);
    if !path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("failed to read browser state {}", path.display()))?;
    let state = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse browser state {}", path.display()))?;
    Ok(Some(state))
}

fn browser_supervisor_state_path(paths: &IkarosPaths, profile: &str) -> PathBuf {
    browser_supervisor_dir(paths).join(format!("{}.json", clean_browser_profile(profile)))
}

fn browser_supervisor_state_path_for_state(state: &BrowserSupervisorState) -> PathBuf {
    state
        .log_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{}.json", clean_browser_profile(&state.profile)))
}

fn browser_supervisor_dir(paths: &IkarosPaths) -> PathBuf {
    paths.home.join("browser").join("supervisor")
}

fn browser_profile_dir(paths: &IkarosPaths, profile: &str) -> PathBuf {
    paths
        .home
        .join("browser")
        .join("profiles")
        .join(clean_browser_profile(profile))
}

fn clean_browser_profile(profile: &str) -> String {
    let cleaned = profile
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    let cleaned = cleaned.trim_matches('-');
    if cleaned.is_empty() {
        "default".into()
    } else {
        cleaned.to_owned()
    }
}

fn default_browser_binary() -> PathBuf {
    std::env::var_os("IKAROS_BROWSER")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            if cfg!(target_os = "macos") {
                PathBuf::from("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
            } else if cfg!(target_os = "windows") {
                PathBuf::from("chrome.exe")
            } else {
                PathBuf::from("google-chrome")
            }
        })
}

fn validate_browser_process_arg(value: &str) -> Result<()> {
    if value.chars().any(|ch| ch.is_control())
        || value.contains('|')
        || value.contains(';')
        || value.contains('&')
    {
        bail!("browser extra arg contains shell/control characters");
    }
    Ok(())
}

fn browser_pid_is_running(pid: u32) -> bool {
    #[cfg(unix)]
    {
        Command::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }
    #[cfg(windows)]
    {
        Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}")])
            .stdin(Stdio::null())
            .output()
            .map(|output| String::from_utf8_lossy(&output.stdout).contains(&pid.to_string()))
            .unwrap_or(false)
    }
}

fn stop_browser_process(pid: u32) -> Option<String> {
    #[cfg(unix)]
    {
        Command::new("kill")
            .arg(pid.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
            .map(|status| {
                if status.success() {
                    "sigterm-sent".into()
                } else {
                    format!("sigterm-failed:{status}")
                }
            })
    }
    #[cfg(windows)]
    {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok()
            .map(|status| {
                if status.success() {
                    "taskkill-sent".into()
                } else {
                    format!("taskkill-failed:{status}")
                }
            })
    }
}

async fn browser_cdp_cli_output(
    session: &ExecutionSession,
    command: &BrowserCommand,
) -> Result<Option<Value>> {
    let output = match command {
        BrowserCommand::Navigate(args) => {
            validate_browser_target_url(&args.url)?;
            browser_cdp_action_json(
                session,
                "navigate",
                &args.cdp.endpoint,
                &args.target_id,
                vec![
                    cdp_command("Page.enable", json!({})),
                    cdp_command("Page.navigate", json!({ "url": &args.url })),
                ],
                None,
            )
            .await?
        }
        BrowserCommand::Snapshot(args) => {
            browser_cdp_action_json(
                session,
                "snapshot",
                &args.cdp.endpoint,
                &args.target_id,
                vec![cdp_command(
                    "Runtime.evaluate",
                    json!({
                        "expression": "(() => ({ title: document.title, url: location.href, text: document.body ? document.body.innerText.slice(0, 20000) : '', html: document.documentElement ? document.documentElement.outerHTML.slice(0, 20000) : '' }))()",
                        "returnByValue": true,
                        "awaitPromise": true,
                    }),
                )],
                None,
            )
            .await?
        }
        BrowserCommand::Click(args) => {
            browser_cdp_action_json(
                session,
                "click",
                &args.cdp.endpoint,
                &args.target_id,
                vec![
                    cdp_command("Input.dispatchMouseEvent", json!({"type": "mouseMoved", "x": args.x, "y": args.y, "button": "none"})),
                    cdp_command("Input.dispatchMouseEvent", json!({"type": "mousePressed", "x": args.x, "y": args.y, "button": "left", "clickCount": 1})),
                    cdp_command("Input.dispatchMouseEvent", json!({"type": "mouseReleased", "x": args.x, "y": args.y, "button": "left", "clickCount": 1})),
                ],
                None,
            )
            .await?
        }
        BrowserCommand::Type(args) => {
            browser_cdp_action_json(
                session,
                "type",
                &args.cdp.endpoint,
                &args.target_id,
                vec![cdp_command("Input.insertText", json!({ "text": &args.text }))],
                None,
            )
            .await?
        }
        BrowserCommand::Scroll(args) => {
            browser_cdp_action_json(
                session,
                "scroll",
                &args.cdp.endpoint,
                &args.target_id,
                vec![cdp_command(
                    "Runtime.evaluate",
                    json!({
                        "expression": format!("window.scrollBy({}, {}); true;", args.x, args.y),
                        "returnByValue": true,
                    }),
                )],
                None,
            )
            .await?
        }
        BrowserCommand::Screenshot(args) => {
            let format = browser_screenshot_format(&args.format)?;
            browser_cdp_action_json(
                session,
                "screenshot",
                &args.cdp.endpoint,
                &args.target_id,
                vec![cdp_command("Page.captureScreenshot", json!({ "format": format }))],
                Some("screenshot_base64_redacted_in_logs"),
            )
            .await?
        }
        BrowserCommand::Cdp(args) => {
            let params = serde_json::from_str::<Value>(&args.params_json)
                .map(redact_json)
                .map_err(|error| anyhow::anyhow!("--params-json must be valid JSON: {error}"))?;
            browser_cdp_action_json(
                session,
                "cdp",
                &args.cdp.endpoint,
                &args.target_id,
                vec![cdp_command(&args.method, params)],
                None,
            )
            .await?
        }
        _ => return Ok(None),
    };
    Ok(Some(output))
}

pub(crate) async fn run_browser_workbench_command(
    session: &ExecutionSession,
    paths: &IkarosPaths,
    args: &[&str],
) -> Result<()> {
    if matches!(args, ["help"] | ["--help"]) {
        print_browser_workbench_usage();
        return Ok(());
    }
    if let Some(output) = browser_supervisor_workbench_output(paths, args)? {
        println!("browser_json: {}", serde_json::to_string(&output)?);
        return Ok(());
    }
    if let Some(output) = browser_cdp_workbench_output(session, args).await? {
        println!("browser_json: {}", serde_json::to_string(&output)?);
        return Ok(());
    }
    let request = parse_browser_workbench_request(args)?;
    let url = cdp_endpoint_url(&request.endpoint, &request.path);
    let response = send_cdp_request(session, request.method, &url).await?;
    let output = browser_response_json(
        request.schema,
        request.action,
        &request.endpoint,
        &url,
        request.target_url_policy,
        &response,
        session.audit.path().display().to_string(),
    );
    println!("browser_json: {}", serde_json::to_string(&output)?);
    Ok(())
}

fn browser_supervisor_workbench_output(
    paths: &IkarosPaths,
    args: &[&str],
) -> Result<Option<Value>> {
    let Some(command) = args.first().copied() else {
        return Ok(None);
    };
    match command {
        "launch" => {
            let launch = parse_browser_launch_workbench_args(&args[1..])?;
            Ok(Some(launch_browser_supervisor(paths, &launch)?))
        }
        "supervisor-status" | "supervisor" => {
            let profile = parse_browser_profile_arg(&args[1..])?;
            Ok(Some(browser_supervisor_status(paths, &profile, "status")?))
        }
        "stop" => {
            let profile = parse_browser_profile_arg(&args[1..])?;
            Ok(Some(stop_browser_supervisor(paths, &profile)?))
        }
        _ => Ok(None),
    }
}

fn parse_browser_launch_workbench_args(args: &[&str]) -> Result<BrowserLaunchArgs> {
    let mut browser = None;
    let mut remote_debugging_port = 9222;
    let mut user_data_dir = None;
    let mut headless = false;
    let mut profile = "default".to_owned();
    let mut url = None;
    let mut extra_args = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--browser" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("usage: /browser launch --browser PATH"))?;
                browser = Some(PathBuf::from(value));
                index += 2;
            }
            "--remote-debugging-port" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow::anyhow!("usage: /browser launch --remote-debugging-port PORT")
                })?;
                remote_debugging_port = value
                    .parse::<u16>()
                    .with_context(|| "--remote-debugging-port must be a TCP port")?;
                index += 2;
            }
            "--user-data-dir" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    anyhow::anyhow!("usage: /browser launch --user-data-dir PATH")
                })?;
                user_data_dir = Some(PathBuf::from(value));
                index += 2;
            }
            "--headless" => {
                headless = true;
                index += 1;
            }
            "--profile" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("usage: /browser launch --profile NAME"))?;
                profile = (*value).to_owned();
                index += 2;
            }
            "--url" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("usage: /browser launch --url URL"))?;
                url = Some((*value).to_owned());
                index += 2;
            }
            "--" => {
                extra_args.extend(args[index + 1..].iter().map(|value| (*value).to_owned()));
                break;
            }
            value if value.starts_with("--") => {
                return Err(anyhow::anyhow!("unknown /browser launch argument: {value}"));
            }
            value if url.is_none() => {
                url = Some(value.to_owned());
                index += 1;
            }
            value => {
                extra_args.push(value.to_owned());
                index += 1;
            }
        }
    }
    Ok(BrowserLaunchArgs {
        browser,
        remote_debugging_port,
        user_data_dir,
        headless,
        profile,
        url,
        extra_args,
    })
}

fn parse_browser_profile_arg(args: &[&str]) -> Result<String> {
    let mut profile = "default".to_owned();
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--profile" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("usage: /browser stop --profile NAME"))?;
                profile = (*value).to_owned();
                index += 2;
            }
            "help" | "--help" => {
                print_browser_workbench_usage();
                index += 1;
            }
            value => {
                return Err(anyhow::anyhow!(
                    "unknown /browser supervisor argument: {value}"
                ));
            }
        }
    }
    Ok(profile)
}

async fn browser_cdp_workbench_output(
    session: &ExecutionSession,
    args: &[&str],
) -> Result<Option<Value>> {
    let Some(command) = args.first().copied() else {
        return Ok(None);
    };
    let rest = &args[1..];
    let output = match command {
        "navigate" => {
            let (endpoint, values) = parse_browser_endpoint_and_values(rest)?;
            let target_id = values
                .first()
                .ok_or_else(|| anyhow::anyhow!("usage: /browser navigate <target-id> <url>"))?;
            let url = values
                .get(1)
                .ok_or_else(|| anyhow::anyhow!("usage: /browser navigate <target-id> <url>"))?;
            validate_browser_target_url(url)?;
            browser_cdp_action_json(
                session,
                "navigate",
                &endpoint,
                target_id,
                vec![
                    cdp_command("Page.enable", json!({})),
                    cdp_command("Page.navigate", json!({ "url": url })),
                ],
                None,
            )
            .await?
        }
        "snapshot" => {
            let (endpoint, values) = parse_browser_endpoint_and_values(rest)?;
            let target_id = values
                .first()
                .ok_or_else(|| anyhow::anyhow!("usage: /browser snapshot <target-id>"))?;
            browser_cdp_action_json(
                session,
                "snapshot",
                &endpoint,
                target_id,
                vec![cdp_command(
                    "Runtime.evaluate",
                    json!({
                        "expression": "(() => ({ title: document.title, url: location.href, text: document.body ? document.body.innerText.slice(0, 20000) : '', html: document.documentElement ? document.documentElement.outerHTML.slice(0, 20000) : '' }))()",
                        "returnByValue": true,
                        "awaitPromise": true,
                    }),
                )],
                None,
            )
            .await?
        }
        "click" => {
            let (endpoint, values) = parse_browser_endpoint_and_values(rest)?;
            let target_id = values
                .first()
                .ok_or_else(|| anyhow::anyhow!("usage: /browser click <target-id> <x> <y>"))?;
            let x = parse_f64_arg(values.get(1), "x")?;
            let y = parse_f64_arg(values.get(2), "y")?;
            browser_cdp_action_json(
                session,
                "click",
                &endpoint,
                target_id,
                vec![
                    cdp_command("Input.dispatchMouseEvent", json!({"type": "mouseMoved", "x": x, "y": y, "button": "none"})),
                    cdp_command("Input.dispatchMouseEvent", json!({"type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1})),
                    cdp_command("Input.dispatchMouseEvent", json!({"type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1})),
                ],
                None,
            )
            .await?
        }
        "type" => {
            let (endpoint, values) = parse_browser_endpoint_and_values(rest)?;
            let target_id = values
                .first()
                .ok_or_else(|| anyhow::anyhow!("usage: /browser type <target-id> <text>"))?;
            let text = values
                .get(1..)
                .map(|values| values.join(" "))
                .filter(|text| !text.is_empty())
                .ok_or_else(|| anyhow::anyhow!("usage: /browser type <target-id> <text>"))?;
            browser_cdp_action_json(
                session,
                "type",
                &endpoint,
                target_id,
                vec![cdp_command("Input.insertText", json!({ "text": text }))],
                None,
            )
            .await?
        }
        "scroll" => {
            let (endpoint, values) = parse_browser_endpoint_and_values(rest)?;
            let target_id = values
                .first()
                .ok_or_else(|| anyhow::anyhow!("usage: /browser scroll <target-id> [x] [y]"))?;
            let x = values
                .get(1)
                .map(|value| value.parse::<f64>())
                .transpose()
                .with_context(|| "scroll x must be numeric")?
                .unwrap_or(0.0);
            let y = values
                .get(2)
                .map(|value| value.parse::<f64>())
                .transpose()
                .with_context(|| "scroll y must be numeric")?
                .unwrap_or(600.0);
            browser_cdp_action_json(
                session,
                "scroll",
                &endpoint,
                target_id,
                vec![cdp_command(
                    "Runtime.evaluate",
                    json!({
                        "expression": format!("window.scrollBy({}, {}); true;", x, y),
                        "returnByValue": true,
                    }),
                )],
                None,
            )
            .await?
        }
        "screenshot" => {
            let (endpoint, values) = parse_browser_endpoint_and_values(rest)?;
            let target_id = values.first().ok_or_else(|| {
                anyhow::anyhow!("usage: /browser screenshot <target-id> [png|jpeg|webp]")
            })?;
            let format =
                browser_screenshot_format(values.get(1).map(String::as_str).unwrap_or("png"))?;
            browser_cdp_action_json(
                session,
                "screenshot",
                &endpoint,
                target_id,
                vec![cdp_command(
                    "Page.captureScreenshot",
                    json!({ "format": format }),
                )],
                Some("screenshot_base64_redacted_in_logs"),
            )
            .await?
        }
        "cdp" => {
            let (endpoint, values) = parse_browser_endpoint_and_values(rest)?;
            let target_id = values.first().ok_or_else(|| {
                anyhow::anyhow!("usage: /browser cdp <target-id> <method> [params-json]")
            })?;
            let method = values.get(1).ok_or_else(|| {
                anyhow::anyhow!("usage: /browser cdp <target-id> <method> [params-json]")
            })?;
            let params = values
                .get(2)
                .map(|value| serde_json::from_str::<Value>(value))
                .transpose()
                .with_context(|| "params-json must be valid JSON")?
                .unwrap_or_else(|| json!({}));
            browser_cdp_action_json(
                session,
                "cdp",
                &endpoint,
                target_id,
                vec![cdp_command(method, params)],
                None,
            )
            .await?
        }
        _ => return Ok(None),
    };
    Ok(Some(output))
}

#[derive(Debug, Clone)]
struct BrowserCdpCommand {
    method: String,
    params: Value,
}

fn cdp_command(method: impl Into<String>, params: Value) -> BrowserCdpCommand {
    BrowserCdpCommand {
        method: method.into(),
        params,
    }
}

async fn browser_cdp_action_json(
    session: &ExecutionSession,
    action: &str,
    endpoint: &str,
    target_id: &str,
    commands: Vec<BrowserCdpCommand>,
    note: Option<&str>,
) -> Result<Value> {
    let websocket_url = resolve_cdp_websocket_url(session, endpoint, target_id).await?;
    let responses = send_cdp_commands(&websocket_url, &commands).await?;
    Ok(redact_json(json!({
        "schema": "ikaros-browser-cdp-action-v1",
        "version": 1,
        "action": action,
        "endpoint": redact_secrets(endpoint),
        "target_id": redact_secrets(target_id),
        "websocket_transport": "direct_cdp",
        "websocket_url": redact_secrets(&websocket_url),
        "commands": commands.iter().map(|command| redact_json(json!({
            "method": &command.method,
            "params": &command.params,
        }))).collect::<Vec<_>>(),
        "responses": responses,
        "note": note,
        "audit": session.audit.path().display().to_string(),
    })))
}

async fn resolve_cdp_websocket_url(
    session: &ExecutionSession,
    endpoint: &str,
    target_id: &str,
) -> Result<String> {
    if target_id.starts_with("ws://") || target_id.starts_with("wss://") {
        return Ok(target_id.to_owned());
    }
    validate_target_id(target_id)?;
    let list_url = cdp_endpoint_url(endpoint, "/json/list");
    let response = send_cdp_request(session, "GET", &list_url).await?;
    let targets = serde_json::from_str::<Value>(&response.body)
        .with_context(|| "CDP /json/list response was not valid JSON")?;
    let targets = targets
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("CDP /json/list response must be a JSON array"))?;
    for target in targets {
        let id = target
            .get("id")
            .and_then(Value::as_str)
            .or_else(|| target.get("targetId").and_then(Value::as_str));
        if id == Some(target_id)
            && let Some(url) = target.get("webSocketDebuggerUrl").and_then(Value::as_str)
        {
            return Ok(url.to_owned());
        }
    }
    anyhow::bail!("CDP target not found: {}", redact_secrets(target_id))
}

async fn send_cdp_commands(
    websocket_url: &str,
    commands: &[BrowserCdpCommand],
) -> Result<Vec<Value>> {
    let (mut socket, _) = connect_async(websocket_url).await.with_context(|| {
        format!(
            "failed to connect CDP websocket {}",
            redact_secrets(websocket_url)
        )
    })?;
    let mut responses = Vec::new();
    for (index, command) in commands.iter().enumerate() {
        let id = index + 1;
        let request = json!({
            "id": id,
            "method": &command.method,
            "params": &command.params,
        });
        socket
            .send(Message::Text(serde_json::to_string(&request)?.into()))
            .await
            .with_context(|| format!("failed to send CDP command {}", command.method))?;
        loop {
            let Some(message) = socket.next().await else {
                anyhow::bail!(
                    "CDP websocket closed before response for {}",
                    command.method
                );
            };
            let message = message.with_context(|| "failed to read CDP websocket message")?;
            let text = match message {
                Message::Text(text) => text.to_string(),
                Message::Binary(bytes) => String::from_utf8_lossy(bytes.as_ref()).into_owned(),
                Message::Close(_) => {
                    anyhow::bail!(
                        "CDP websocket closed before response for {}",
                        command.method
                    );
                }
                _ => continue,
            };
            let value = serde_json::from_str::<Value>(&text)
                .map(redact_json)
                .unwrap_or_else(|_| json!({ "raw": redact_secrets(&text) }));
            if value.get("id").and_then(Value::as_u64) == Some(id as u64) {
                responses.push(redact_cdp_response(value));
                break;
            }
        }
    }
    let _ = socket.close(None).await;
    Ok(responses)
}

fn redact_cdp_response(value: Value) -> Value {
    let screenshot_data = value
        .pointer("/result/data")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let mut value = redact_json(value);
    if let Some(data) = screenshot_data
        && looks_like_base64_image(&data)
    {
        value["result"]["data"] = json!({
            "redacted": true,
            "kind": "base64_image",
            "bytes_estimate": base64::engine::general_purpose::STANDARD
                .decode(data.as_bytes())
                .map(|bytes| bytes.len())
                .unwrap_or_else(|_| data.len() * 3 / 4),
        });
    }
    value
}

fn looks_like_base64_image(value: &str) -> bool {
    value.len() > 1024
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '+' | '/' | '=' | '\n' | '\r')
        })
}

fn browser_screenshot_format(format: &str) -> Result<&'static str> {
    match format {
        "png" => Ok("png"),
        "jpeg" | "jpg" => Ok("jpeg"),
        "webp" => Ok("webp"),
        value => anyhow::bail!("unsupported screenshot format: {value}"),
    }
}

struct BrowserWorkbenchRequest {
    schema: &'static str,
    action: &'static str,
    method: &'static str,
    path: String,
    endpoint: String,
    target_url_policy: Option<&'static str>,
}

fn parse_browser_workbench_request(args: &[&str]) -> Result<BrowserWorkbenchRequest> {
    let command = args.first().copied().unwrap_or("status");
    let rest = if args.is_empty() { &[][..] } else { &args[1..] };
    match command {
        "status" => {
            let endpoint = parse_browser_endpoint(rest)?;
            Ok(BrowserWorkbenchRequest {
                schema: "ikaros-browser-cdp-status-v1",
                action: "status",
                method: "GET",
                path: "/json/version".into(),
                endpoint,
                target_url_policy: None,
            })
        }
        "list" => {
            let endpoint = parse_browser_endpoint(rest)?;
            Ok(BrowserWorkbenchRequest {
                schema: "ikaros-browser-cdp-list-v1",
                action: "list",
                method: "GET",
                path: "/json/list".into(),
                endpoint,
                target_url_policy: None,
            })
        }
        "new" => {
            let (endpoint, values) = parse_browser_endpoint_and_values(rest)?;
            let target_url = values.first().map(String::as_str).unwrap_or("about:blank");
            Ok(BrowserWorkbenchRequest {
                schema: "ikaros-browser-cdp-new-target-v1",
                action: "new",
                method: "PUT",
                path: cdp_new_target_path(target_url)?,
                endpoint,
                target_url_policy: Some("cdp_endpoint_governed_target_page_loaded_by_browser"),
            })
        }
        "activate" | "close" => {
            let (endpoint, values) = parse_browser_endpoint_and_values(rest)?;
            let target_id = values
                .first()
                .ok_or_else(|| anyhow::anyhow!("usage: /browser {command} <target-id>"))?;
            let (schema, action, path) = if command == "activate" {
                (
                    "ikaros-browser-cdp-activate-target-v1",
                    "activate",
                    cdp_target_path("/json/activate", target_id)?,
                )
            } else {
                (
                    "ikaros-browser-cdp-close-target-v1",
                    "close",
                    cdp_target_path("/json/close", target_id)?,
                )
            };
            Ok(BrowserWorkbenchRequest {
                schema,
                action,
                method: "GET",
                path,
                endpoint,
                target_url_policy: None,
            })
        }
        "help" | "--help" => {
            print_browser_workbench_usage();
            Err(anyhow::anyhow!("browser usage printed"))
        }
        value => Err(anyhow::anyhow!("unsupported /browser command: {value}")),
    }
}

fn parse_browser_endpoint(args: &[&str]) -> Result<String> {
    let (endpoint, values) = parse_browser_endpoint_and_values(args)?;
    if !values.is_empty() {
        anyhow::bail!("unexpected /browser argument: {}", values[0]);
    }
    Ok(endpoint)
}

fn parse_browser_endpoint_and_values(args: &[&str]) -> Result<(String, Vec<String>)> {
    let mut endpoint = DEFAULT_CDP_ENDPOINT.to_owned();
    let mut values = Vec::new();
    let mut index = 0;
    while index < args.len() {
        match args[index] {
            "--endpoint" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| anyhow::anyhow!("usage: /browser ... --endpoint URL"))?;
                endpoint = (*value).to_owned();
                index += 2;
            }
            "help" | "--help" => {
                print_browser_workbench_usage();
                index += 1;
            }
            value => {
                values.push(value.to_owned());
                index += 1;
            }
        }
    }
    Ok((endpoint, values))
}

fn parse_f64_arg(value: Option<&String>, name: &str) -> Result<f64> {
    value
        .ok_or_else(|| anyhow::anyhow!("missing numeric browser argument: {name}"))?
        .parse::<f64>()
        .with_context(|| format!("{name} must be numeric"))
}

fn print_browser_workbench_usage() {
    println!(
        "browser_usage: /browser [launch [url] [--profile NAME] [--headless]|supervisor-status [--profile NAME]|stop [--profile NAME]|status|list|new <url>|activate <target-id>|close <target-id>|navigate <target-id> <url>|snapshot <target-id>|click <target-id> <x> <y>|type <target-id> <text>|scroll <target-id> [x] [y]|screenshot <target-id>|cdp <target-id> <method> [params-json]] [--endpoint URL]"
    );
}

async fn send_cdp_request(
    session: &ExecutionSession,
    method: &str,
    url: &str,
) -> Result<NetworkEgressResponse> {
    Ok(session
        .env
        .send_network_request(NetworkEgressRequest {
            method: method.into(),
            url: url.into(),
            headers: BTreeMap::new(),
            body: None,
            body_bytes: None,
        })
        .await?)
}

fn browser_response_json(
    schema: &str,
    action: &str,
    endpoint: &str,
    url: &str,
    target_url_policy: Option<&str>,
    response: &NetworkEgressResponse,
    audit: String,
) -> serde_json::Value {
    let parsed = serde_json::from_str::<serde_json::Value>(&response.body)
        .ok()
        .map(redact_json);
    let body_preview = parsed
        .is_none()
        .then(|| truncated_redacted_body(&response.body));
    let output = json!({
        "schema": schema,
        "version": 1,
        "action": action,
        "endpoint": redact_secrets(endpoint),
        "url": redact_secrets(url),
        "http_status": response.status,
        "headers": redacted_headers(&response.headers),
        "json": parsed,
        "body_preview": body_preview,
        "target_url_policy": target_url_policy,
        "audit": audit,
    });
    output
}

fn cdp_endpoint_url(endpoint: &str, path: &str) -> String {
    format!("{}{}", endpoint.trim_end_matches('/'), path)
}

fn cdp_new_target_path(target_url: &str) -> Result<String> {
    validate_browser_target_url(target_url)?;
    let encoded = byte_serialize(target_url.as_bytes()).collect::<String>();
    Ok(format!("/json/new?{encoded}"))
}

fn cdp_target_path(prefix: &str, target_id: &str) -> Result<String> {
    validate_target_id(target_id)?;
    Ok(format!("{prefix}/{target_id}"))
}

fn validate_browser_target_url(target_url: &str) -> Result<()> {
    if target_url == "about:blank" {
        return Ok(());
    }
    let parsed = Url::parse(target_url)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!(
            "browser target URL scheme is unsupported: {}",
            redact_secrets(parsed.scheme())
        );
    }
    if parsed.host_str().is_none() {
        bail!("browser target URL must include a host");
    }
    Ok(())
}

fn validate_target_id(target_id: &str) -> Result<()> {
    if target_id.is_empty()
        || target_id.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || character == '-' || character == '_')
        })
    {
        bail!("browser target id contains unsupported characters");
    }
    Ok(())
}

fn truncated_redacted_body(body: &str) -> String {
    const MAX_CHARS: usize = 2048;
    let redacted = redact_secrets(body);
    let mut chars = redacted.chars();
    let mut output = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        output.push_str("\n[truncated]");
    }
    output
}

fn redacted_headers(headers: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    headers
        .iter()
        .map(|(key, value)| (key.clone(), redact_secrets(value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdp_endpoint_url_normalizes_slashes() {
        assert_eq!(
            cdp_endpoint_url("http://127.0.0.1:9222/", "/json/version"),
            "http://127.0.0.1:9222/json/version"
        );
    }

    #[test]
    fn truncated_body_redacts_secret_like_text() {
        let preview = truncated_redacted_body("token sk-browser-secret");
        assert!(!preview.contains("sk-browser-secret"));
        assert!(preview.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn cdp_new_target_path_encodes_target_url() {
        assert_eq!(
            cdp_new_target_path("https://example.com/a b?q=one&x=two").expect("path"),
            "/json/new?https%3A%2F%2Fexample.com%2Fa+b%3Fq%3Done%26x%3Dtwo"
        );
    }

    #[test]
    fn cdp_target_path_rejects_path_injection() {
        assert!(cdp_target_path("/json/activate", "../target").is_err());
        assert_eq!(
            cdp_target_path("/json/activate", "ABC_123-def").expect("path"),
            "/json/activate/ABC_123-def"
        );
    }
}
