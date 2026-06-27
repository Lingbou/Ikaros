// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::{RiskLevel, ToolCall, ToolResult, now_rfc3339};
use serde_json::json;
use std::fs;

#[test]
fn approval_log_persists_pending_requests_with_redaction() {
    let temp = tempfile::tempdir().expect("tempdir");
    let log = ApprovalLog::new(temp.path());
    let request = ApprovalRequest {
        id: "approval-1".into(),
        call: ToolCall::new(
            "fs_write_guarded",
            RiskLevel::LocalWrite,
            json!({"path": "note.txt", "content": "token=abc123"}),
        ),
        reason: "write requires approval".into(),
        created_at: now_rfc3339().expect("time"),
        workspace_root: Some(temp.path().join("workspace")),
        context: Some(json!({"note": "token=abc123"})),
    };
    log.append_request(request).expect("append");
    let pending = log.pending().expect("pending");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].request.id, "approval-1");
    assert_eq!(
        pending[0].request.call.input["content"],
        json!("token=[REDACTED_SECRET]")
    );
    assert_eq!(
        pending[0].request.context.as_ref().expect("context")["note"],
        json!("token=[REDACTED_SECRET]")
    );
    let execution_request = log
        .execution_request("approval-1")
        .expect("execution request")
        .expect("stored execution request");
    assert_eq!(
        execution_request.call.input["content"],
        json!("token=abc123")
    );
    let raw = fs::read_to_string(log.path()).expect("approval log");
    assert!(!raw.contains("abc123"));
    assert!(raw.contains("[REDACTED_SECRET]"));
}

#[test]
fn approval_log_rejects_replayed_decisions_after_terminal_status() {
    let temp = tempfile::tempdir().expect("tempdir");
    let log = ApprovalLog::new(temp.path());
    let request = approval_request(temp.path().join("workspace"));
    log.append_request(request).expect("append");
    log.decide("approval-1", ApprovalStatus::Denied, None)
        .expect("deny");

    let error = log
        .decide("approval-1", ApprovalStatus::Approved, None)
        .expect_err("replay should fail");
    assert!(error.to_string().contains("not pending"));
}

#[test]
fn approval_log_rejects_replayed_execution_and_redacts_result() {
    let temp = tempfile::tempdir().expect("tempdir");
    let log = ApprovalLog::new(temp.path());
    let request = approval_request(temp.path().join("workspace"));
    log.append_request(request).expect("append");
    log.decide(
        "approval-1",
        ApprovalStatus::Approved,
        Some("operator note token=abc123".into()),
    )
    .expect("approve");
    log.mark_executed(
        "approval-1",
        ToolResult {
            call_id: "call-1".into(),
            ok: true,
            output: json!({
                "stdout": "printed sk-not-real",
                "nested": {"token": "abc123"},
            }),
            summary: "completed with password=hunter2".into(),
        },
    )
    .expect("execute");

    let error = log
        .decide("approval-1", ApprovalStatus::Approved, None)
        .expect_err("decision replay should fail");
    assert!(error.to_string().contains("not pending"));
    let error = log
        .mark_executed(
            "approval-1",
            ToolResult {
                call_id: "call-1".into(),
                ok: true,
                output: json!({}),
                summary: "repeat".into(),
            },
        )
        .expect_err("execution replay should fail");
    assert!(error.to_string().contains("not approved"));

    let raw = fs::read_to_string(log.path()).expect("approval log");
    assert!(!raw.contains("abc123"));
    assert!(!raw.contains("sk-not-real"));
    assert!(!raw.contains("hunter2"));
    assert!(raw.contains("[REDACTED_SECRET]"));
}

fn approval_request(workspace_root: std::path::PathBuf) -> ApprovalRequest {
    ApprovalRequest {
        id: "approval-1".into(),
        call: ToolCall {
            id: "call-1".into(),
            name: "fs_write_guarded".into(),
            risk: RiskLevel::LocalWrite,
            input: json!({"path": "note.txt", "content": "hello"}),
        },
        reason: "write requires approval".into(),
        created_at: now_rfc3339().expect("time"),
        workspace_root: Some(workspace_root),
        context: None,
    }
}
