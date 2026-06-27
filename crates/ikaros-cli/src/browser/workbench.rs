// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result};
use ikaros_core::IkarosPaths;
use ikaros_execution::harness::ExecutionSession;
use serde_json::{Value, json};
use std::path::PathBuf;

use super::cdp::{browser_cdp_action_json, browser_screenshot_format, cdp_command};
use super::http::{
    browser_response_json, cdp_endpoint_url, cdp_new_target_path, cdp_target_path,
    send_cdp_request, validate_browser_target_url,
};
use super::supervisor::{
    browser_supervisor_status, launch_browser_supervisor, stop_browser_supervisor,
};
use super::{BrowserLaunchArgs, DEFAULT_CDP_ENDPOINT};
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
