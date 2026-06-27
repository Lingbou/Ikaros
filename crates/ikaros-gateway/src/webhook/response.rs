// SPDX-License-Identifier: GPL-3.0-only

use super::{
    acl::MessageWebhookAcl,
    payload::{parse_webhook_pairing_code, parse_webhook_payload},
};
use crate::LocalGatewayStore;
use ikaros_core::{Result, redact_secrets};
use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageWebhookHttpResponse {
    pub status_code: u16,
    pub reason: &'static str,
    pub content_type: &'static str,
    pub body: String,
    pub allow: Option<&'static str>,
}

impl MessageWebhookHttpResponse {
    pub fn plain(status_code: u16, reason: &'static str, body: impl Into<String>) -> Self {
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

#[derive(Debug, Clone, Copy, Default)]
pub struct MessageWebhookIngressPolicy<'a> {
    pub acl: Option<&'a MessageWebhookAcl>,
    pub require_pairing: bool,
    pub safe_tools: bool,
}

pub fn webhook_http_response(
    method: &str,
    route: &str,
    content_type: Option<&str>,
    body: &[u8],
    store: &LocalGatewayStore,
    policy: MessageWebhookIngressPolicy<'_>,
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
            let mut route = match parse_webhook_payload(body) {
                Ok(route) => route,
                Err(error) => {
                    return MessageWebhookHttpResponse::json(
                        400,
                        "Bad Request",
                        json!({"ok": false, "error": redact_secrets(&error)}),
                    );
                }
            };
            if let Some(acl) = policy.acl {
                if let Err(field) = acl.validate_route(&route) {
                    return Ok(MessageWebhookHttpResponse::plain(
                        403,
                        "Forbidden",
                        format!("webhook ACL rejected {field}\n"),
                    ));
                }
            }
            if policy.require_pairing
                && !store.route_has_paired_peer(&route)?
                && !confirm_webhook_pairing(store, &route, body)?
            {
                return Ok(MessageWebhookHttpResponse::plain(
                    403,
                    "Forbidden",
                    "webhook pairing required\n",
                ));
            }
            if policy.safe_tools {
                route = route.with_safe_tools(true);
            }
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

pub fn is_message_route(route: &str) -> bool {
    matches!(route, "/message" | "/messages")
}

fn confirm_webhook_pairing(
    store: &LocalGatewayStore,
    route: &crate::GatewayRoute,
    body: &[u8],
) -> Result<bool> {
    let Some(code) = parse_webhook_pairing_code(body) else {
        return Ok(false);
    };
    Ok(store.confirm_pairing_for_route(route, &code)?.is_some())
}
