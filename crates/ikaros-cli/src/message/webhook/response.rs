// SPDX-License-Identifier: GPL-3.0-only

use super::payload::parse_webhook_payload;
use anyhow::Result;
use ikaros_core::redact_secrets;
use ikaros_gateway::LocalGatewayStore;
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MessageWebhookHttpResponse {
    pub(super) status_code: u16,
    pub(super) reason: &'static str,
    pub(super) content_type: &'static str,
    pub(super) body: String,
    pub(super) allow: Option<&'static str>,
}

impl MessageWebhookHttpResponse {
    pub(super) fn plain(status_code: u16, reason: &'static str, body: impl Into<String>) -> Self {
        Self {
            status_code,
            reason,
            content_type: "text/plain; charset=utf-8",
            body: body.into(),
            allow: None,
        }
    }

    fn json(status_code: u16, reason: &'static str, body: Value) -> Result<Self> {
        Ok(Self {
            status_code,
            reason,
            content_type: "application/json; charset=utf-8",
            body: serde_json::to_string_pretty(&body)?,
            allow: None,
        })
    }

    fn method_not_allowed(allow: &'static str) -> Self {
        Self {
            status_code: 405,
            reason: "Method Not Allowed",
            content_type: "text/plain; charset=utf-8",
            body: "method not allowed\n".into(),
            allow: Some(allow),
        }
    }
}

pub(super) fn webhook_http_response(
    method: &str,
    route: &str,
    content_type: Option<&str>,
    body: &[u8],
    store: &LocalGatewayStore,
) -> Result<MessageWebhookHttpResponse> {
    match (method, route) {
        ("GET" | "HEAD", "/healthz") => Ok(MessageWebhookHttpResponse::plain(200, "OK", "ok\n")),
        ("GET" | "HEAD", "/") => Ok(MessageWebhookHttpResponse::plain(
            200,
            "OK",
            "POST JSON to /message with content, optional kind, source, and profile\n",
        )),
        ("POST", route) if is_message_route(route) => {
            if !content_type
                .unwrap_or("application/json")
                .to_ascii_lowercase()
                .starts_with("application/json")
            {
                return Ok(MessageWebhookHttpResponse::plain(
                    415,
                    "Unsupported Media Type",
                    "expected application/json\n",
                ));
            }
            let route = match parse_webhook_payload(body) {
                Ok(route) => route,
                Err(error) => {
                    return MessageWebhookHttpResponse::json(
                        400,
                        "Bad Request",
                        json!({"ok": false, "error": redact_secrets(&error)}),
                    );
                }
            };
            let message = store.enqueue(route)?;
            MessageWebhookHttpResponse::json(
                202,
                "Accepted",
                json!({
                    "ok": true,
                    "message": message,
                    "gateway_inbox": store.inbox_path(),
                }),
            )
        }
        (_, route) if is_message_route(route) => {
            Ok(MessageWebhookHttpResponse::method_not_allowed("POST"))
        }
        _ => Ok(MessageWebhookHttpResponse::plain(
            404,
            "Not Found",
            "not found\n",
        )),
    }
}

pub(super) fn is_message_route(route: &str) -> bool {
    matches!(route, "/message" | "/messages")
}
