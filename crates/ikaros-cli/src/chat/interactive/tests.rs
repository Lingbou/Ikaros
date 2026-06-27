// SPDX-License-Identifier: GPL-3.0-only

use super::{parse_timeline_request, parse_trace_request};

#[test]
fn parses_timeline_kind_page_and_turn_in_any_order() {
    let request = parse_timeline_request(vec!["--kind", "model", "--page", "2", "turn-one"])
        .expect("timeline request");

    assert_eq!(request.turn_filter.as_deref(), Some("turn-one"));
    assert_eq!(request.kind_filter.as_deref(), Some("model"));
    assert_eq!(request.page, 2);
}

#[test]
fn parses_timeline_failed_and_approval_point_filters() {
    let failed = parse_timeline_request(vec!["turn-one", "--failed"]).expect("failed request");
    assert_eq!(failed.turn_filter.as_deref(), Some("turn-one"));
    assert_eq!(failed.point_filter.as_deref(), Some("failed"));

    let approval =
        parse_timeline_request(vec!["--approval", "--page", "3"]).expect("approval request");
    assert_eq!(approval.point_filter.as_deref(), Some("approval"));
    assert_eq!(approval.page, 3);
}

#[test]
fn rejects_unknown_timeline_kind() {
    let error = parse_timeline_request(vec!["--kind", "missing"]).expect_err("timeline error");

    assert!(error.to_string().contains("unknown timeline kind"));
}

#[test]
fn parses_trace_kind_turn_and_point_filters() {
    let request = parse_trace_request(vec!["turn-one", "--kind", "coding", "--failed"])
        .expect("trace request");

    assert_eq!(request.turn_filter.as_deref(), Some("turn-one"));
    assert_eq!(request.kind_filter.as_deref(), Some("coding"));
    assert_eq!(request.point_filter.as_deref(), Some("failed"));
}
