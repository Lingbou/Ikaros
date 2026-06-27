// SPDX-License-Identifier: GPL-3.0-only

use super::{
    DEFAULT_CDP_ENDPOINT,
    actions::{BrowserHttpAction, BrowserWsAction, browser_action_name, browser_ws_action_name},
};
use ikaros_core::{IkarosError, Result, redact_secrets};
use serde_json::Value;
use url::{Url, form_urlencoded::byte_serialize};

pub(in crate::browser) fn required_target_id(input: &Value) -> Result<String> {
    let Some(target_id) = input
        .get("target_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Err(IkarosError::Message("browser target_id is required".into()));
    };
    Ok(target_id.to_owned())
}

pub(in crate::browser) fn browser_endpoint(input: &Value) -> String {
    input
        .get("endpoint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_CDP_ENDPOINT)
        .to_owned()
}

pub(in crate::browser) fn browser_policy_command(
    action: BrowserHttpAction,
    input: &Value,
) -> String {
    let endpoint = browser_endpoint(input);
    match action {
        BrowserHttpAction::New => format!(
            "action=new endpoint={} url={}",
            redact_secrets(&endpoint),
            input
                .get("url")
                .and_then(Value::as_str)
                .map(redact_secrets)
                .unwrap_or_else(|| "about:blank".into())
        ),
        BrowserHttpAction::Activate | BrowserHttpAction::Close => format!(
            "action={} endpoint={} target_id={}",
            browser_action_name(action),
            redact_secrets(&endpoint),
            input
                .get("target_id")
                .and_then(Value::as_str)
                .map(redact_secrets)
                .unwrap_or_else(|| "missing".into())
        ),
        BrowserHttpAction::Status | BrowserHttpAction::List => {
            format!(
                "action={} endpoint={}",
                browser_action_name(action),
                redact_secrets(&endpoint)
            )
        }
    }
}

pub(in crate::browser) fn browser_action_request(
    action: BrowserHttpAction,
    input: &Value,
) -> Result<(&'static str, String)> {
    match action {
        BrowserHttpAction::Status => Ok(("GET", "/json/version".into())),
        BrowserHttpAction::List => Ok(("GET", "/json/list".into())),
        BrowserHttpAction::New => {
            let url = input
                .get("url")
                .and_then(Value::as_str)
                .unwrap_or("about:blank");
            validate_browser_target_url(url)?;
            Ok(("PUT", cdp_new_target_path(url)))
        }
        BrowserHttpAction::Activate => {
            let target_id = required_target_id(input)?;
            Ok(("GET", cdp_target_path("/json/activate", &target_id)?))
        }
        BrowserHttpAction::Close => {
            let target_id = required_target_id(input)?;
            Ok(("GET", cdp_target_path("/json/close", &target_id)?))
        }
    }
}

pub(in crate::browser) fn cdp_endpoint_url(endpoint: &str, path: &str) -> Result<String> {
    let base = Url::parse(endpoint)
        .map_err(|_| IkarosError::Message("browser endpoint must be a valid URL".into()))?;
    if !matches!(base.scheme(), "http" | "https") {
        return Err(IkarosError::Message(format!(
            "browser endpoint scheme is unsupported: {}",
            redact_secrets(base.scheme())
        )));
    }
    if base.host_str().is_none() {
        return Err(IkarosError::Message(
            "browser endpoint must include a host".into(),
        ));
    }
    let mut normalized = endpoint.trim_end_matches('/').to_owned();
    normalized.push('/');
    normalized.push_str(path.trim_start_matches('/'));
    Ok(normalized)
}

pub(in crate::browser) fn cdp_new_target_path(target_url: &str) -> String {
    let encoded = byte_serialize(target_url.as_bytes()).collect::<String>();
    format!("/json/new?{encoded}")
}

pub(in crate::browser) fn cdp_target_path(prefix: &str, target_id: &str) -> Result<String> {
    if !target_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(IkarosError::Message(
            "browser target_id contains unsupported characters".into(),
        ));
    }
    Ok(format!("{}/{}", prefix.trim_end_matches('/'), target_id))
}

pub(in crate::browser) fn validate_browser_target_url(target_url: &str) -> Result<()> {
    if target_url == "about:blank" {
        return Ok(());
    }
    let parsed = Url::parse(target_url)
        .map_err(|_| IkarosError::Message("browser target URL must be valid".into()))?;
    if !matches!(parsed.scheme(), "http" | "https" | "file") {
        return Err(IkarosError::Message(format!(
            "browser target URL scheme is unsupported: {}",
            redact_secrets(parsed.scheme())
        )));
    }
    Ok(())
}

pub(in crate::browser) fn input_number(input: &Value, field: &str) -> Result<f64> {
    optional_input_number(input, field)
        .ok_or_else(|| IkarosError::Message(format!("browser {field} must be numeric")))
}

pub(in crate::browser) fn optional_input_number(input: &Value, field: &str) -> Option<f64> {
    input.get(field).and_then(Value::as_f64)
}

pub(in crate::browser) fn validate_target_id(target_id: &str) -> Result<()> {
    if !target_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(IkarosError::Message(
            "browser target_id contains unsupported characters".into(),
        ));
    }
    Ok(())
}

pub(in crate::browser) fn browser_ws_policy_command(
    action: BrowserWsAction,
    input: &Value,
) -> String {
    let endpoint = browser_endpoint(input);
    let target_id = input
        .get("target_id")
        .and_then(Value::as_str)
        .map(redact_secrets)
        .unwrap_or_else(|| "missing".into());
    match action {
        BrowserWsAction::Navigate => format!(
            "action=navigate endpoint={} target_id={} url={}",
            redact_secrets(&endpoint),
            target_id,
            input
                .get("url")
                .and_then(Value::as_str)
                .map(redact_secrets)
                .unwrap_or_else(|| "missing".into())
        ),
        BrowserWsAction::Click => format!(
            "action=click endpoint={} target_id={} x={} y={}",
            redact_secrets(&endpoint),
            target_id,
            input.get("x").and_then(Value::as_f64).unwrap_or_default(),
            input.get("y").and_then(Value::as_f64).unwrap_or_default()
        ),
        BrowserWsAction::Type => format!(
            "action=type endpoint={} target_id={} text_chars={}",
            redact_secrets(&endpoint),
            target_id,
            input
                .get("text")
                .and_then(Value::as_str)
                .map(|text| text.chars().count())
                .unwrap_or_default()
        ),
        BrowserWsAction::Screenshot => format!(
            "action=screenshot endpoint={} target_id={} output_path={}",
            redact_secrets(&endpoint),
            target_id,
            input
                .get("output_path")
                .and_then(Value::as_str)
                .map(redact_secrets)
                .unwrap_or_else(|| "none".into())
        ),
        _ => format!(
            "action={} endpoint={} target_id={}",
            browser_ws_action_name(action),
            redact_secrets(&endpoint),
            target_id
        ),
    }
}
