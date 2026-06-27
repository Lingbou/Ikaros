// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result, bail};
use ikaros_core::{IkarosPaths, now_rfc3339, redact_json};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use super::http::validate_browser_target_url;
use super::{BrowserCommand, BrowserLaunchArgs};
pub(super) fn browser_supervisor_cli_output(
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

pub(super) fn launch_browser_supervisor(
    paths: &IkarosPaths,
    args: &BrowserLaunchArgs,
) -> Result<Value> {
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

pub(super) fn browser_supervisor_status(
    paths: &IkarosPaths,
    profile: &str,
    action: &str,
) -> Result<Value> {
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

pub(super) fn stop_browser_supervisor(paths: &IkarosPaths, profile: &str) -> Result<Value> {
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
