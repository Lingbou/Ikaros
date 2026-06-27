// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use ikaros_core::{redact_json, redact_secrets};
use ikaros_execution::harness::ExecutionSession;
use serde_json::{Value, json};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::BrowserCommand;
use super::http::{
    cdp_endpoint_url, send_cdp_request, validate_browser_target_url, validate_target_id,
};
pub(super) async fn browser_cdp_cli_output(
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

#[derive(Debug, Clone)]
pub(super) struct BrowserCdpCommand {
    method: String,
    params: Value,
}

pub(super) fn cdp_command(method: impl Into<String>, params: Value) -> BrowserCdpCommand {
    BrowserCdpCommand {
        method: method.into(),
        params,
    }
}

pub(super) async fn browser_cdp_action_json(
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

pub(super) fn browser_screenshot_format(format: &str) -> Result<&'static str> {
    match format {
        "png" => Ok("png"),
        "jpeg" | "jpg" => Ok("jpeg"),
        "webp" => Ok("webp"),
        value => anyhow::bail!("unsupported screenshot format: {value}"),
    }
}
