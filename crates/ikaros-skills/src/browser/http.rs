// SPDX-License-Identifier: GPL-3.0-only

use super::{
    actions::{BrowserHttpAction, browser_action_name},
    util::{browser_action_request, browser_endpoint, cdp_endpoint_url},
};
use ikaros_core::{Result, redact_json, redact_secrets};
use ikaros_toolkit::{NetworkEgressRequest, SkillContext, SkillOutput};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub(in crate::browser) async fn execute_browser_http_action(
    action: BrowserHttpAction,
    input: Value,
    ctx: SkillContext,
) -> Result<SkillOutput> {
    let endpoint = browser_endpoint(&input);
    let (method, path) = browser_action_request(action, &input)?;
    let url = cdp_endpoint_url(&endpoint, &path)?;
    let response = ctx
        .session
        .env
        .send_network_request(NetworkEgressRequest {
            method: method.into(),
            url: url.clone(),
            headers: BTreeMap::from([("accept".into(), "application/json".into())]),
            body: None,
            body_bytes: None,
        })
        .await?;
    let body = response
        .body_bytes
        .as_deref()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .unwrap_or(response.body);
    let parsed = serde_json::from_str::<Value>(&body)
        .map(redact_json)
        .unwrap_or_else(|_| json!({"text": redact_secrets(&body)}));
    Ok(SkillOutput::new(
        format!(
            "browser {} status={} endpoint={}",
            browser_action_name(action),
            response.status,
            redact_secrets(&endpoint)
        ),
        json!({
            "schema": "ikaros-browser-skill-result-v1",
            "action": browser_action_name(action),
            "endpoint": redact_secrets(&endpoint),
            "url": redact_secrets(&url),
            "status": response.status,
            "ok": (200..300).contains(&response.status),
            "body": parsed,
        }),
    ))
}
