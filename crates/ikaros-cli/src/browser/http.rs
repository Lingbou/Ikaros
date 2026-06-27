// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Result, bail};
use ikaros_core::{redact_json, redact_secrets};
use ikaros_execution::harness::ExecutionSession;
use ikaros_host::{BrowserCdpHttpResponse, send_browser_cdp_http_request};
use serde_json::json;
use std::collections::BTreeMap;
use url::{Url, form_urlencoded::byte_serialize};
pub(super) async fn send_cdp_request(
    session: &ExecutionSession,
    method: &str,
    url: &str,
) -> Result<BrowserCdpHttpResponse> {
    Ok(send_browser_cdp_http_request(session, method, url).await?)
}

pub(super) fn browser_response_json(
    schema: &str,
    action: &str,
    endpoint: &str,
    url: &str,
    target_url_policy: Option<&str>,
    response: &BrowserCdpHttpResponse,
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

pub(super) fn cdp_endpoint_url(endpoint: &str, path: &str) -> String {
    format!("{}{}", endpoint.trim_end_matches('/'), path)
}

pub(super) fn cdp_new_target_path(target_url: &str) -> Result<String> {
    validate_browser_target_url(target_url)?;
    let encoded = byte_serialize(target_url.as_bytes()).collect::<String>();
    Ok(format!("/json/new?{encoded}"))
}

pub(super) fn cdp_target_path(prefix: &str, target_id: &str) -> Result<String> {
    validate_target_id(target_id)?;
    Ok(format!("{prefix}/{target_id}"))
}

pub(super) fn validate_browser_target_url(target_url: &str) -> Result<()> {
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

pub(super) fn validate_target_id(target_id: &str) -> Result<()> {
    if target_id.is_empty()
        || target_id.chars().any(|character| {
            !(character.is_ascii_alphanumeric() || character == '-' || character == '_')
        })
    {
        bail!("browser target id contains unsupported characters");
    }
    Ok(())
}

pub(super) fn truncated_redacted_body(body: &str) -> String {
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
