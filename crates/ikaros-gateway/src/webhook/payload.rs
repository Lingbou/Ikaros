// SPDX-License-Identifier: GPL-3.0-only

use crate::{GatewayMessageKind, GatewayRoute, GatewaySessionSource};
use serde_json::Value;

pub fn parse_webhook_payload(body: &[u8]) -> std::result::Result<GatewayRoute, String> {
    let value: Value =
        serde_json::from_slice(body).map_err(|error| format!("invalid json body: {error}"))?;
    let content = string_field(&value, "content")
        .or_else(|| string_field(&value, "text"))
        .ok_or_else(|| "missing string field: content".to_string())?;
    if content.trim().is_empty() {
        return Err("message content cannot be empty".into());
    }
    let kind = match string_field(&value, "kind")
        .unwrap_or("chat")
        .to_ascii_lowercase()
        .as_str()
    {
        "chat" => GatewayMessageKind::Chat,
        "task" => GatewayMessageKind::Task,
        other => return Err(format!("unsupported message kind: {other}")),
    };
    let source = string_field(&value, "source").unwrap_or("webhook");
    let agent = string_field(&value, "profile")
        .or_else(|| string_field(&value, "agent"))
        .map(ToOwned::to_owned);
    let mut route = GatewayRoute::new(source, kind, content, agent);
    let account = string_field(&value, "account").map(ToOwned::to_owned);
    let peer = string_field(&value, "peer").map(ToOwned::to_owned);
    let thread = string_field(&value, "thread").map(ToOwned::to_owned);
    let message_id = string_field(&value, "message_id").map(ToOwned::to_owned);
    if account.is_some() || peer.is_some() || thread.is_some() || message_id.is_some() {
        route = route.with_session_source(GatewaySessionSource {
            channel: source.to_owned(),
            account,
            peer,
            thread,
            message_id,
        });
    }
    if let Some(idempotency_key) = string_field(&value, "idempotency_key") {
        route = route.with_idempotency_key(idempotency_key);
    }
    Ok(route)
}

pub fn parse_webhook_pairing_code(body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(body).ok()?;
    string_field(&value, "pairing_code")
        .filter(|code| !code.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn string_field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    match value.get(name) {
        Some(Value::String(value)) => Some(value.as_str()),
        Some(Value::Null) | None => None,
        _ => None,
    }
}
