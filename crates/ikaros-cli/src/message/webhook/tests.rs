// SPDX-License-Identifier: GPL-3.0-only

use super::{
    http::parse_http_request_line, payload::parse_webhook_payload, response::webhook_http_response,
};
use ikaros_gateway::{GatewayMessageKind, LocalGatewayStore};

#[test]
fn parses_webhook_payload_with_defaults_and_redaction() {
    let route = parse_webhook_payload(br#"{"content":"hello api_key=abc123"}"#).expect("payload");
    assert_eq!(route.source, "webhook");
    assert_eq!(route.kind, GatewayMessageKind::Chat);
    assert!(!route.content.contains("abc123"));
    assert!(route.content.contains("[REDACTED_SECRET]"));
}

#[test]
fn parses_webhook_payload_task_profile_and_alias_text() {
    let route = parse_webhook_payload(
        br#"{"text":"summarize project","kind":"task","source":"telegram","profile":"plan"}"#,
    )
    .expect("payload");
    assert_eq!(route.source, "telegram");
    assert_eq!(route.kind, GatewayMessageKind::Task);
    assert_eq!(route.content, "summarize project");
    assert_eq!(route.agent.as_deref(), Some("plan"));
}

#[test]
fn rejects_invalid_webhook_payloads() {
    assert!(parse_webhook_payload(br#"{"kind":"chat"}"#).is_err());
    assert!(parse_webhook_payload(br#"{"content":"   "}"#).is_err());
    assert!(parse_webhook_payload(br#"{"content":"x","kind":"write"}"#).is_err());
}

#[test]
fn parses_simple_http_request_line() {
    assert_eq!(
        parse_http_request_line("POST /message HTTP/1.1\r\n"),
        Some(("POST", "/message"))
    );
    assert_eq!(parse_http_request_line("POST /message\r\n"), None);
}

#[test]
fn webhook_http_response_enqueues_redacted_message() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalGatewayStore::new(temp.path());
    let response = webhook_http_response(
        "POST",
        "/message",
        Some("application/json"),
        br#"{"content":"hello token=abc123","kind":"chat"}"#,
        &store,
    )
    .expect("response");
    assert_eq!(response.status_code, 202);
    let messages = store.list().expect("messages");
    assert_eq!(messages.len(), 1);
    assert!(!messages[0].content.contains("abc123"));
}
