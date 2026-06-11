// SPDX-License-Identifier: GPL-3.0-only

use ikaros_gateway::{GatewayMessageKind, GatewayRoute};
use serde_json::Value;

pub(super) fn parse_webhook_payload(body: &[u8]) -> std::result::Result<GatewayRoute, String> {
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
    Ok(GatewayRoute::new(source, kind, content, agent))
}

fn string_field<'a>(value: &'a Value, name: &str) -> Option<&'a str> {
    match value.get(name) {
        Some(Value::String(value)) => Some(value.as_str()),
        Some(Value::Null) | None => None,
        _ => None,
    }
}
