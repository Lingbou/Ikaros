// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_core::IkarosPaths;
use ikaros_state::session::{
    AgentEvent, AgentEventKind, EventId, SessionEntry, SessionEntryKind, SessionId, TurnId,
};
use ikaros_terminal::{terminal_inline, terminal_message};
use std::fs;
use tempfile::tempdir;

#[test]
fn workbench_history_redacts_secret_like_input() {
    let temp = tempdir().expect("tempdir");
    let paths = IkarosPaths::from_home(temp.path());

    let path = append_workbench_history(&paths, "token sk-secret-value").expect("append");

    let raw = fs::read_to_string(path).expect("history");
    assert!(!raw.contains("sk-secret-value"));
    assert!(raw.contains("[REDACTED_SECRET]"));
}

#[test]
fn workbench_history_preserves_multiline_entries_without_secret_leakage() {
    let temp = tempdir().expect("tempdir");
    let paths = IkarosPaths::from_home(temp.path());

    append_workbench_history(&paths, "first\napi_key=sk-secret-value\nthird").expect("append");

    let entries = load_workbench_history_entries(&paths, 10).expect("history entries");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0], "first\n[REDACTED_SECRET]\nthird");
}

#[test]
fn terminal_inline_redacts_and_replaces_control_characters() {
    let rendered = terminal_inline("token sk-secret-value\nnext\tcell\r");

    assert!(!rendered.contains("sk-secret-value"));
    assert!(!rendered.chars().any(char::is_control));
    assert_eq!(rendered, "token [REDACTED_SECRET]_next_cell_");
}

#[test]
fn terminal_message_strips_mouse_escape_sequences() {
    let rendered = terminal_message("\u{1b}[<35;55;37Mhello\u{1b}[<35;55;37m");
    assert_eq!(rendered, "hello");
}

#[test]
fn terminal_message_strips_bare_mouse_escape_fragments() {
    let rendered = terminal_message("[<35;55;37Mhello[<35;55;37m");
    assert_eq!(rendered, "hello");
}

#[test]
fn terminal_message_strips_double_open_mouse_escape_fragments() {
    let rendered = terminal_message("[[<35;55;37Mhello[[<35;55;37m");
    assert_eq!(rendered, "hello");
}

#[test]
fn terminal_message_drops_partial_mouse_escape_tail() {
    let rendered = terminal_message("hello[<35;55");
    assert_eq!(rendered, "hello");
}

#[test]
fn session_ids_are_normalized_for_terminal_resume() {
    assert_eq!(
        normalize_session_id("gateway/thread:one\n"),
        "gateway_thread_one"
    );
}

#[test]
fn session_entry_cells_redact_visible_text() {
    let mut entry = SessionEntry::new(
        SessionId::from("session-one"),
        SessionEntryKind::UserMessage,
    );
    entry.turn_id = Some(TurnId::from("turn-one"));
    entry.visible_text = Some("use key sk-secret-value".into());

    let rendered = session_entry_cell(&entry).render();

    assert!(rendered.contains("kind=session"));
    assert!(rendered.contains("turn=turn-one"));
    assert!(!rendered.contains("sk-secret-value"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
}

#[test]
fn agent_event_cells_group_coding_events() {
    let event = AgentEvent {
        event_id: EventId::from("event-one"),
        session_id: SessionId::from("session-one"),
        turn_id: TurnId::from("turn-one"),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_state::session::AgentEventSource::Runtime,
        kind: AgentEventKind::CodingTurn,
        payload: serde_json::Value::Null,
    };

    let rendered = agent_event_cell(&event).render();

    assert!(rendered.contains("kind=coding"));
    assert!(rendered.contains("coding_turn"));
    assert!(rendered.contains("turn=turn-one"));
}

#[test]
fn agent_event_cell_renders_tool_progress_payload_without_secret_leakage() {
    let event = AgentEvent {
        event_id: EventId::from("event-tool"),
        session_id: SessionId::from("session-tool"),
        turn_id: TurnId::from("turn-tool"),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_state::session::AgentEventSource::Tool,
        kind: AgentEventKind::ToolCallFailed,
        payload: serde_json::json!({
            "tool_call_id": "call-123",
            "tool_event_id": "event-started",
            "name": "shell_exec",
            "status": "failed",
            "ok": false,
            "execution_mode": "sequential",
            "timeout_ms": 5000,
            "summary": "command failed with api_key=sk-secret-value",
            "output": {
                "error": "exit 1 token=sk-secret-value",
                "recoverable": true
            }
        }),
    };

    let rendered = agent_event_cell(&event).render();

    assert!(rendered.contains("kind=tool"));
    assert!(rendered.contains("tool_call_failed"));
    assert!(rendered.contains("tool=shell_exec"));
    assert!(rendered.contains("status=failed"));
    assert!(rendered.contains("call=call-123"));
    assert!(rendered.contains("mode=sequential"));
    assert!(rendered.contains("timeout_ms=5000"));
    assert!(rendered.contains("summary=command failed"));
    assert!(!rendered.contains("sk-secret-value"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
}

#[test]
fn agent_event_cell_renders_continuation_payload_without_secret_leakage() {
    let event = AgentEvent {
        event_id: EventId::from("event-continuation"),
        session_id: SessionId::from("session-continuation"),
        turn_id: TurnId::from("turn-continuation"),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_state::session::AgentEventSource::Runtime,
        kind: AgentEventKind::ContinuationCancelled,
        payload: serde_json::json!({
            "continuation_id": "continuation-one",
            "kind": "next_turn",
            "status": "cancelled",
            "reason": "operator cancelled token=sk-secret-value",
            "attempt_count": 2,
        }),
    };

    let rendered = agent_event_cell(&event).render();

    assert!(rendered.contains("kind=continuation"));
    assert!(rendered.contains("continuation_cancelled"));
    assert!(rendered.contains("continuation_id=continuation-one"));
    assert!(rendered.contains("continuation_kind=next_turn"));
    assert!(rendered.contains("status=cancelled"));
    assert!(rendered.contains("reason=operator cancelled"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(rendered.contains("attempts=2"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn agent_event_cell_renders_context_progress_payload_without_secret_leakage() {
    let event = AgentEvent {
        event_id: EventId::from("event-context"),
        session_id: SessionId::from("session-context"),
        turn_id: TurnId::from("turn-context"),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_state::session::AgentEventSource::Context,
        kind: AgentEventKind::ContextDiff,
        payload: serde_json::json!({
            "budget": {
                "used_tokens": 1024,
                "max_tokens": 4096,
                "context_window": 8192,
                "estimator": "heuristic-v1"
            },
            "sections": [
                {"kind": "history"},
                {"kind": "references", "lines": ["token sk-secret-value"]}
            ],
            "references": [
                {"raw": "@file:src/lib.rs:1-4"}
            ],
            "compression_summary": "compressed token sk-secret-value",
            "continuation_prompt": "continue"
        }),
    };

    let rendered = agent_event_cell(&event).render();

    assert!(rendered.contains("kind=context"));
    assert!(rendered.contains("context_diff"));
    assert!(rendered.contains("sections=2"));
    assert!(rendered.contains("references=1"));
    assert!(rendered.contains("used=1024"));
    assert!(rendered.contains("max=4096"));
    assert!(rendered.contains("context_window=8192"));
    assert!(rendered.contains("estimator=heuristic-v1"));
    assert!(rendered.contains("compressed=yes"));
    assert!(rendered.contains("continuation_prompt=yes"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn agent_event_cell_renders_model_stream_markdown_without_secret_leakage() {
    let event = AgentEvent {
        event_id: EventId::from("event-model"),
        session_id: SessionId::from("session-model"),
        turn_id: TurnId::from("turn-model"),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_state::session::AgentEventSource::Model,
        kind: AgentEventKind::ModelStream(
            ikaros_providers::model::ModelStreamEvent::TextDelta(
                "Here is code:\n\n```rust\nlet token = \"sk-secret-value\";\n```\n\n| File | Status |\n| --- | --- |\n| src/lib.rs | changed |\n".into(),
            ),
        ),
        payload: serde_json::Value::Null,
    };

    let rendered = agent_event_cell(&event).render();

    assert!(rendered.contains("kind=model"));
    assert!(rendered.contains("model_stream"));
    assert!(rendered.contains("stream_event=text_delta"));
    assert!(rendered.contains("--- rust"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(rendered.contains("File"));
    assert!(rendered.contains("Status"));
    assert!(rendered.contains("src/lib.rs"));
    assert!(rendered.contains("changed"));
    assert!(!rendered.contains("[code"));
    assert!(!rendered.contains("[table]"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn agent_event_cell_renders_error_phase_message_and_recovery_actions() {
    let event = AgentEvent {
        event_id: EventId::from("event-error"),
        session_id: SessionId::from("session-error"),
        turn_id: TurnId::from("turn-error"),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_state::session::AgentEventSource::Runtime,
        kind: AgentEventKind::Error,
        payload: serde_json::json!({
            "phase": "model_call",
            "message": "temporary failure in name resolution api_key=sk-secret-value"
        }),
    };

    let rendered = agent_event_cell(&event).render();

    assert!(rendered.contains("kind=error"));
    assert!(rendered.contains("error"));
    assert!(rendered.contains("phase=model_call"));
    assert!(rendered.contains("kind=provider_error"));
    assert!(rendered.contains("/provider debug"));
    assert!(rendered.contains("/provider health --live"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn coding_event_cells_group_diff_test_review_and_progress() {
    let events = [
        "diff_updated",
        "test_evidence_recorded",
        "review_completed",
        "plan_prepared",
    ]
    .into_iter()
    .map(|kind| AgentEvent {
        event_id: EventId::new(),
        session_id: SessionId::from("session-one"),
        turn_id: TurnId::from("turn-one"),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_state::session::AgentEventSource::Tool,
        kind: AgentEventKind::CodingTurn,
        payload: serde_json::json!({
            "kind": kind,
            "summary": format!("summary for {kind}"),
        }),
    })
    .collect::<Vec<_>>();

    let cells = coding_event_cells(&events);
    let groups = cells.iter().map(|(group, _)| *group).collect::<Vec<_>>();

    assert_eq!(groups, vec!["diff", "test", "review", "progress"]);
    assert!(
        cells
            .iter()
            .any(|(_, cell)| cell.render().contains("title=coding diff"))
    );
}

#[test]
fn workbench_snapshot_wraps_cells_and_redacts_secret_text() {
    let snapshot = render_workbench_snapshot(
        &[
            WorkbenchCell {
                kind: WorkbenchCellKind::Coding,
                title: "coding diff".into(),
                detail: "turn=turn-one kind=diff_updated summary=changed src/lib.rs with sk-secret-value".into(),
            },
            WorkbenchCell {
                kind: WorkbenchCellKind::Approval,
                title: "approval pending".into(),
                detail: "provider_call=true workspace_write=true shell=false".into(),
            },
        ],
        42,
    );

    assert_eq!(
        snapshot,
        "snapshot width=42\n[coding] coding diff\n  turn=turn-one kind=diff_updated\n  summary=changed src/lib.rs with\n  [REDACTED_SECRET]\n[approval] approval pending\n  provider_call=true workspace_write=true\n  shell=false\n"
    );
}

#[test]
fn workbench_snapshot_splits_long_tokens_at_width() {
    let snapshot = render_workbench_snapshot(
        &[WorkbenchCell {
            kind: WorkbenchCellKind::Context,
            title: "context reference".into(),
            detail: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        }],
        18,
    );

    for line in snapshot.lines().skip(1) {
        assert!(
            line.chars().count() <= 18,
            "snapshot line exceeds width: {line}"
        );
    }
}

#[test]
fn fullscreen_workbench_frame_renders_core_panels_without_secret_leakage() {
    let screen = WorkbenchScreen {
        title: "Ikaros Workbench".into(),
        status: vec![
            WorkbenchCell {
                kind: WorkbenchCellKind::Model,
                title: "model".into(),
                detail: "provider=openai-compatible model=kimi-k2.6".into(),
            },
            WorkbenchCell {
                kind: WorkbenchCellKind::Continuation,
                title: "queue".into(),
                detail: "running=1 pending=2".into(),
            },
        ],
        timeline: vec![WorkbenchCell {
            kind: WorkbenchCellKind::Coding,
            title: "coding test failed".into(),
            detail: "turn=turn-one cargo test failed".into(),
        }],
        main: vec![WorkbenchCell {
            kind: WorkbenchCellKind::Context,
            title: "context".into(),
            detail: "budget=8192 references=2".into(),
        }],
        side: vec![WorkbenchCell {
            kind: WorkbenchCellKind::Approval,
            title: "approval pending".into(),
            detail: "write=true shell=true token=sk-secret-value".into(),
        }],
        footer: "session=session-one /approval approve <id>".into(),
        input_hint: "/code apply --run-tests".into(),
    };

    let frame =
        render_fullscreen_workbench_with_state(&screen, &WorkbenchScreenState::default(), 72, 18);

    assert!(frame.contains("Ikaros"));
    assert!(frame.contains("kimi-k2.6"));
    assert!(frame.contains("Ask Ikaros to do anything"));
    assert!(!frame.contains("Timeline"));
    assert!(!frame.contains("Approvals / Queue"));
    assert!(!frame.contains("selected panel="));
    assert!(!frame.contains("actions selector="));
    assert!(!frame.contains("sk-secret-value"));
    for line in frame.lines() {
        assert!(
            line.chars().count() <= 72,
            "frame line exceeds width: {line}"
        );
    }
    assert_eq!(frame.lines().count(), 18);
}

#[test]
fn workbench_history_entries_load_redacted_multiline_records() {
    let temp = tempdir().expect("tempdir");
    let paths = IkarosPaths::from_home(temp.path());
    append_workbench_history(&paths, "first message").expect("append first");
    append_workbench_history(&paths, "second token=sk-secret-value").expect("append second");

    let entries = load_workbench_history_entries(&paths, 8).expect("history entries");

    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0], "first message");
    assert!(entries[1].contains("second"));
    assert!(!entries[1].contains("sk-secret-value"));
    assert!(entries[1].contains("[REDACTED_SECRET]"));
}

#[test]
fn agent_event_cell_renders_model_diagnostic_kind_and_message() {
    let event = AgentEvent {
        event_id: EventId::from("event-diag"),
        session_id: SessionId::from("session-diag"),
        turn_id: TurnId::from("turn-diag"),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_state::session::AgentEventSource::Model,
        kind: AgentEventKind::ModelDiagnostic(ikaros_providers::model::ModelRequestDiagnostic {
            kind: "fallback_provider_selected".into(),
            message: "provider openai-compatible/qwen-2.5-72b selected after 1 fallback attempt(s)"
                .into(),
            parameter: None,
        }),
        payload: serde_json::Value::Null,
    };

    let rendered = agent_event_cell(&event).render();

    assert!(rendered.contains("kind=model"));
    assert!(rendered.contains("model_diagnostic"));
    assert!(rendered.contains("turn=turn-diag"));
    assert!(rendered.contains("diagnostic=fallback_provider_selected"));
    assert!(rendered.contains("qwen-2.5-72b"));
}
