// SPDX-License-Identifier: GPL-3.0-only

use super::{
    dashboard::{dashboard_output_path, dashboard_snapshot_href, ikaros_home_output_path},
    server::{dashboard_http_response, parse_http_request_line},
};
use ikaros_body::BodyEventKind;
use ikaros_core::IkarosPaths;
use ikaros_harness::{AuditEvent, ExecutionSession};
use ikaros_runtime::audit_event_to_body_event;
use serde_json::json;
use std::path::PathBuf;

#[test]
fn dashboard_output_path_stays_under_home() {
    let paths = IkarosPaths::from_home("/tmp/ikaros-home");
    let path =
        dashboard_output_path(&paths, Some(PathBuf::from("previews/status.html"))).expect("path");
    assert_eq!(path, PathBuf::from("/tmp/ikaros-home/previews/status.html"));
    let snapshot =
        ikaros_home_output_path(&paths, PathBuf::from("previews/frame.json")).expect("path");
    assert_eq!(
        snapshot,
        PathBuf::from("/tmp/ikaros-home/previews/frame.json")
    );
    assert_eq!(
        dashboard_snapshot_href(&paths.home, &path, &snapshot).expect("href"),
        "frame.json"
    );
    assert_eq!(
        dashboard_snapshot_href(
            &paths.home,
            &PathBuf::from("/tmp/ikaros-home/status.html"),
            &snapshot
        )
        .expect("href"),
        "previews/frame.json"
    );
    assert_eq!(
        dashboard_snapshot_href(
            &paths.home,
            &PathBuf::from("/tmp/ikaros-home/previews/status.html"),
            &PathBuf::from("/tmp/ikaros-home/frame.json")
        )
        .expect("href"),
        "../frame.json"
    );
    assert!(dashboard_output_path(&paths, Some(PathBuf::from("../status.html"))).is_err());
    assert!(dashboard_output_path(&paths, Some(PathBuf::from(".temp/status.html"))).is_err());
    assert!(dashboard_output_path(&paths, Some(PathBuf::from("/tmp/status.html"))).is_err());
    assert!(ikaros_home_output_path(&paths, PathBuf::from("../frame.json")).is_err());
}

#[test]
fn dashboard_event_mapping_redacts_data_values() {
    let event = AuditEvent::new(
        "tool_call",
        None,
        "tool call token=abc123",
        json!({"call_id": "call-1", "input": "api_key=abc123"}),
    )
    .expect("event");
    let body_event = audit_event_to_body_event(event);
    assert_eq!(body_event.kind, BodyEventKind::Skill);
    assert_eq!(
        body_event.data.get("call_id").map(String::as_str),
        Some("call-1")
    );
    assert!(!body_event.message.contains("abc123"));
    assert!(
        !body_event
            .data
            .values()
            .any(|value| value.contains("abc123"))
    );
}

#[test]
fn dashboard_http_response_serves_html_json_and_health() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = IkarosPaths::from_home(temp.path().join("home"));
    paths.ensure().expect("paths");
    let event = AuditEvent::new(
        "tool_call",
        None,
        "tool call token=abc123",
        json!({"input": "token=abc123"}),
    )
    .expect("event");
    ExecutionSession::new(temp.path(), &paths.audit_dir)
        .audit
        .append(event)
        .expect("audit");

    let html =
        dashboard_http_response("GET", "/", &paths, temp.path(), 5, 2).expect("html response");
    assert_eq!(html.status_code, 200);
    assert_eq!(html.content_type, "text/html; charset=utf-8");
    assert!(html.body.contains("http-equiv=\"refresh\" content=\"2\""));
    assert!(html.body.contains("href=\"/frame.json\""));
    assert!(!html.body.contains("abc123"));

    let json_response = dashboard_http_response("GET", "/frame.json", &paths, temp.path(), 5, 2)
        .expect("json response");
    assert_eq!(json_response.status_code, 200);
    assert_eq!(
        json_response.content_type,
        "application/json; charset=utf-8"
    );
    assert!(json_response.body.contains("\"body\": \"Web\""));
    assert!(!json_response.body.contains("abc123"));
    assert!(json_response.body.contains("[REDACTED_SECRET]"));

    let health =
        dashboard_http_response("HEAD", "/healthz", &paths, temp.path(), 5, 2).expect("health");
    assert_eq!(health.status_code, 200);
    assert_eq!(health.body, "ok\n");

    let missing =
        dashboard_http_response("GET", "/missing", &paths, temp.path(), 5, 2).expect("missing");
    assert_eq!(missing.status_code, 404);

    let denied = dashboard_http_response("POST", "/", &paths, temp.path(), 5, 2).expect("denied");
    assert_eq!(denied.status_code, 405);
    assert!(denied.allow_get_head);
}

#[test]
fn parse_http_request_line_requires_simple_http_request() {
    assert_eq!(
        parse_http_request_line("GET /frame.json HTTP/1.1\r\n"),
        Some(("GET", "/frame.json"))
    );
    assert_eq!(parse_http_request_line("GET /frame.json\r\n"), None);
    assert_eq!(
        parse_http_request_line("GET /frame.json HTTP/1.1 extra\r\n"),
        None
    );
    assert_eq!(parse_http_request_line("GET /frame.json FTP/1.0\r\n"), None);
}
