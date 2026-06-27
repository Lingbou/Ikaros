// SPDX-License-Identifier: GPL-3.0-only

use super::{
    actions::{BrowserWsAction, browser_ws_action_name},
    util::{
        browser_endpoint, cdp_endpoint_url, input_number, optional_input_number,
        required_target_id, validate_browser_target_url, validate_target_id,
    },
};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use ikaros_core::{IkarosError, Result, redact_json, redact_secrets};
use ikaros_tools::{NetworkEgressRequest, SkillContext, SkillOutput};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::Path};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

#[derive(Debug, Clone)]
struct BrowserCdpCommand {
    method: String,
    params: Value,
}

pub(in crate::browser) async fn execute_browser_ws_action(
    action: BrowserWsAction,
    input: Value,
    ctx: SkillContext,
) -> Result<SkillOutput> {
    let endpoint = browser_endpoint(&input);
    let target_id = required_target_id(&input)?;
    let commands = browser_ws_commands(action, &input)?;
    let websocket_url = resolve_cdp_websocket_url(&ctx, &endpoint, &target_id).await?;
    authorize_cdp_websocket(&ctx, &websocket_url).await?;
    let responses = send_cdp_commands(&websocket_url, &commands).await?;
    let saved_screenshot = if action == BrowserWsAction::Screenshot {
        maybe_save_screenshot(&ctx, &input, &responses).await?
    } else {
        None
    };
    let redacted_responses = responses
        .into_iter()
        .map(redact_cdp_response)
        .collect::<Vec<_>>();
    Ok(SkillOutput::new(
        format!(
            "browser {} target={} commands={}",
            browser_ws_action_name(action),
            redact_secrets(&target_id),
            commands.len()
        ),
        json!({
            "schema": "ikaros-browser-cdp-action-v1",
            "version": 1,
            "action": browser_ws_action_name(action),
            "endpoint": redact_secrets(&endpoint),
            "target_id": redact_secrets(&target_id),
            "websocket_transport": "direct_cdp_skill",
            "websocket_url": redact_secrets(&websocket_url),
            "commands": commands.iter().map(|command| redact_json(json!({
                "method": &command.method,
                "params": &command.params,
            }))).collect::<Vec<_>>(),
            "responses": redacted_responses,
            "saved_screenshot": saved_screenshot,
        }),
    ))
}

fn browser_ws_commands(action: BrowserWsAction, input: &Value) -> Result<Vec<BrowserCdpCommand>> {
    match action {
        BrowserWsAction::Navigate => {
            let url = input
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| IkarosError::Message("browser url is required".into()))?;
            validate_browser_target_url(url)?;
            Ok(vec![
                cdp_command("Page.enable", json!({})),
                cdp_command("Page.navigate", json!({ "url": url })),
            ])
        }
        BrowserWsAction::Snapshot => Ok(vec![cdp_command(
            "Runtime.evaluate",
            json!({
                "expression": "(() => ({ title: document.title, url: location.href, text: document.body ? document.body.innerText.slice(0, 8000) : '' }))()",
                "returnByValue": true
            }),
        )]),
        BrowserWsAction::Click => {
            let x = input_number(input, "x")?;
            let y = input_number(input, "y")?;
            Ok(vec![
                cdp_command(
                    "Input.dispatchMouseEvent",
                    json!({"type": "mouseMoved", "x": x, "y": y, "button": "none"}),
                ),
                cdp_command(
                    "Input.dispatchMouseEvent",
                    json!({"type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1}),
                ),
                cdp_command(
                    "Input.dispatchMouseEvent",
                    json!({"type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1}),
                ),
            ])
        }
        BrowserWsAction::Type => {
            let text = input
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| IkarosError::Message("browser text is required".into()))?;
            Ok(vec![cdp_command(
                "Input.insertText",
                json!({ "text": text }),
            )])
        }
        BrowserWsAction::Scroll => {
            let x = optional_input_number(input, "x").unwrap_or(0.0);
            let y = optional_input_number(input, "y").unwrap_or(600.0);
            Ok(vec![cdp_command(
                "Input.dispatchMouseEvent",
                json!({"type": "mouseWheel", "x": 0, "y": 0, "deltaX": x, "deltaY": y}),
            )])
        }
        BrowserWsAction::Screenshot => {
            let format = input.get("format").and_then(Value::as_str).unwrap_or("png");
            let format = browser_screenshot_format(format)?;
            Ok(vec![cdp_command(
                "Page.captureScreenshot",
                json!({ "format": format }),
            )])
        }
        BrowserWsAction::Cdp => {
            let method = input
                .get("method")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| IkarosError::Message("browser CDP method is required".into()))?;
            let params = input.get("params").cloned().unwrap_or_else(|| json!({}));
            Ok(vec![cdp_command(method, params)])
        }
    }
}

fn cdp_command(method: impl Into<String>, params: Value) -> BrowserCdpCommand {
    BrowserCdpCommand {
        method: method.into(),
        params,
    }
}

async fn resolve_cdp_websocket_url(
    ctx: &SkillContext,
    endpoint: &str,
    target_id: &str,
) -> Result<String> {
    if target_id.starts_with("ws://") || target_id.starts_with("wss://") {
        return Ok(target_id.to_owned());
    }
    validate_target_id(target_id)?;
    let list_url = cdp_endpoint_url(endpoint, "/json/list")?;
    let response = ctx
        .session
        .env
        .send_network_request(NetworkEgressRequest {
            method: "GET".into(),
            url: list_url,
            headers: BTreeMap::from([("accept".into(), "application/json".into())]),
            body: None,
            body_bytes: None,
        })
        .await?;
    let targets = serde_json::from_str::<Value>(&response.body).map_err(|source| {
        IkarosError::Message(format!(
            "CDP /json/list response was not valid JSON: {source}"
        ))
    })?;
    let targets = targets.as_array().ok_or_else(|| {
        IkarosError::Message("CDP /json/list response must be a JSON array".into())
    })?;
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
    Err(IkarosError::Message(format!(
        "CDP target not found: {}",
        redact_secrets(target_id)
    )))
}

async fn send_cdp_commands(
    websocket_url: &str,
    commands: &[BrowserCdpCommand],
) -> Result<Vec<Value>> {
    let (mut socket, _) = connect_async(websocket_url).await.map_err(|source| {
        IkarosError::Message(format!(
            "failed to connect CDP websocket {}: {source}",
            redact_secrets(websocket_url)
        ))
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
            .map_err(|source| {
                IkarosError::Message(format!(
                    "failed to send CDP command {}: {source}",
                    redact_secrets(&command.method)
                ))
            })?;
        loop {
            let Some(message) = socket.next().await else {
                return Err(IkarosError::Message(format!(
                    "CDP websocket closed before response for {}",
                    redact_secrets(&command.method)
                )));
            };
            let message = message.map_err(|source| {
                IkarosError::Message(format!("failed to read CDP websocket message: {source}"))
            })?;
            let text = match message {
                Message::Text(text) => text.to_string(),
                Message::Binary(bytes) => String::from_utf8_lossy(bytes.as_ref()).into_owned(),
                Message::Close(_) => {
                    return Err(IkarosError::Message(format!(
                        "CDP websocket closed before response for {}",
                        redact_secrets(&command.method)
                    )));
                }
                _ => continue,
            };
            let value = serde_json::from_str::<Value>(&text)
                .unwrap_or_else(|_| json!({ "raw": redact_secrets(&text) }));
            if value.get("id").and_then(Value::as_u64) == Some(id as u64) {
                responses.push(value);
                break;
            }
        }
    }
    let _ = socket.close(None).await;
    Ok(responses)
}

async fn authorize_cdp_websocket(ctx: &SkillContext, websocket_url: &str) -> Result<()> {
    let preflight_url = cdp_websocket_preflight_url(websocket_url)?;
    ctx.session
        .env
        .send_network_request(NetworkEgressRequest {
            method: "GET".into(),
            url: preflight_url,
            headers: BTreeMap::from([("accept".into(), "application/json".into())]),
            body: None,
            body_bytes: None,
        })
        .await?;
    Ok(())
}

fn cdp_websocket_preflight_url(websocket_url: &str) -> Result<String> {
    let mut parsed = Url::parse(websocket_url)
        .map_err(|_| IkarosError::Message("CDP websocket URL must be valid".into()))?;
    let scheme = match parsed.scheme() {
        "ws" => "http",
        "wss" => "https",
        value => {
            return Err(IkarosError::Message(format!(
                "CDP websocket scheme is unsupported: {}",
                redact_secrets(value)
            )));
        }
    };
    parsed
        .set_scheme(scheme)
        .map_err(|_| IkarosError::Message("failed to build CDP websocket preflight URL".into()))?;
    if parsed.host_str().is_none() {
        return Err(IkarosError::Message(
            "CDP websocket URL must include a host".into(),
        ));
    }
    Ok(parsed.to_string())
}

async fn maybe_save_screenshot(
    ctx: &SkillContext,
    input: &Value,
    responses: &[Value],
) -> Result<Option<Value>> {
    let Some(path) = input
        .get("output_path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let Some(data) = responses
        .iter()
        .find_map(|response| response.pointer("/result/data").and_then(Value::as_str))
    else {
        return Ok(Some(json!({
            "path": redact_secrets(path),
            "saved": false,
            "reason": "missing_screenshot_data",
        })));
    };
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|source| IkarosError::Message(format!("invalid screenshot base64: {source}")))?;
    ctx.session
        .env
        .write_bytes(Path::new(path), bytes.clone())
        .await?;
    Ok(Some(json!({
        "path": redact_secrets(path),
        "saved": true,
        "bytes": bytes.len(),
    })))
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
    match format.trim().to_ascii_lowercase().as_str() {
        "png" => Ok("png"),
        "jpeg" | "jpg" => Ok("jpeg"),
        "webp" => Ok("webp"),
        value => Err(IkarosError::Message(format!(
            "unsupported screenshot format: {value}"
        ))),
    }
}
