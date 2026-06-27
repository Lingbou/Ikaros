// SPDX-License-Identifier: GPL-3.0-only
//! Platform delivery: sends gateway responses to Discord, Slack, and
//! Telegram via webhook URLs through the harness NetworkEgress boundary.

use ikaros_core::{Result, redact_secrets};
use ikaros_gateway::{GatewayOutboundEnvelope, GatewayPlatform};
use ikaros_harness::{NetworkEgress, NetworkEgressRequest};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc};

pub struct PlatformDeliveryReport {
    pub platform: String,
    pub status: u16,
    pub delivered: bool,
    pub error: Option<String>,
}

pub async fn deliver_to_platform(
    platform: GatewayPlatform,
    webhook_url: &str,
    envelope: &GatewayOutboundEnvelope,
    egress: &Arc<dyn NetworkEgress>,
) -> Result<PlatformDeliveryReport> {
    let body = build_platform_payload(platform, envelope);
    let request = NetworkEgressRequest {
        method: "POST".into(),
        url: webhook_url.into(),
        headers: {
            let mut headers = BTreeMap::new();
            headers.insert("content-type".into(), "application/json".into());
            headers
        },
        body: Some(body),
        body_bytes: None,
    };
    let response = egress.send_network_request(request).await?;
    let delivered = response.status >= 200 && response.status < 300;
    let error = if delivered {
        None
    } else {
        Some(format!(
            "HTTP {} {}",
            response.status,
            redact_secrets(&response.body)
        ))
    };
    Ok(PlatformDeliveryReport {
        platform: platform.as_str().into(),
        status: response.status,
        delivered,
        error,
    })
}

fn build_platform_payload(platform: GatewayPlatform, envelope: &GatewayOutboundEnvelope) -> String {
    match platform {
        GatewayPlatform::Discord => serde_json::to_string(&json!({
            "content": truncate(&envelope.content, 1900),
            "username": "Ikaros",
        }))
        .unwrap_or_else(|_| r#"{"content":"delivery failed"}"#.into()),
        GatewayPlatform::Slack => serde_json::to_string(&json!({
            "text": truncate(&envelope.content, 2900),
        }))
        .unwrap_or_else(|_| r#"{"text":"delivery failed"}"#.into()),
        GatewayPlatform::Telegram => serde_json::to_string(&json!({
            "chat_id": envelope.thread.as_deref().unwrap_or(""),
            "text": truncate(&envelope.content, 4000),
            "parse_mode": "Markdown",
        }))
        .unwrap_or_else(|_| r#"{"text":"delivery failed"}"#.into()),
        _ => serde_json::to_string(&json!({
            "content": &envelope.content,
        }))
        .unwrap_or_else(|_| r#"{"content":""}"#.into()),
    }
}

fn truncate(input: &str, max: usize) -> String {
    if input.len() <= max {
        input.to_owned()
    } else {
        let mut truncated = input[..max.saturating_sub(3)].to_owned();
        truncated.push_str("...");
        truncated
    }
}

pub fn platform_webhook_config_key(platform: GatewayPlatform) -> &'static str {
    match platform {
        GatewayPlatform::Discord => "discord_webhook_url",
        GatewayPlatform::Slack => "slack_webhook_url",
        GatewayPlatform::Telegram => "telegram_bot_api_url",
        _ => "webhook_url",
    }
}
