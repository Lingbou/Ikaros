// SPDX-License-Identifier: GPL-3.0-only
//! Stable protocol types shared by CLI, TUI, gateway, API, and replay surfaces.
//!
//! This crate is intentionally small and domain-facing. It contains the durable
//! wire shapes that product surfaces should exchange, not provider adapters,
//! stores, or runtime implementation details.

mod envelope;

mod model;

mod session;

mod trace;

pub use envelope::{IKAROS_PROTOCOL_NAME, IKAROS_PROTOCOL_VERSION, WireEnvelope};

pub use model::{
    MODEL_REQUEST_DIAGNOSTIC_KIND_MAX_CHARS, MODEL_REQUEST_DIAGNOSTIC_MESSAGE_MAX_CHARS,
    MODEL_REQUEST_DIAGNOSTIC_PARAMETER_MAX_CHARS, ModelRequestDiagnostic, ModelStreamEvent,
    TokenUsage,
};

pub use session::*;

pub use trace::{StateTraceEntry, TurnStatus, turn_correlation_id};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn envelope_serializes_session_event() {
        let envelope = WireEnvelope::new(
            "session_event",
            SessionEvent::AssistantDelta(AssistantDeltaEvent {
                session_id: "session-1".into(),
                turn_id: "turn-1".into(),
                message_id: "message-1".into(),
                delta: "hello".into(),
                part_id: Some("part-1".into()),
                part_kind: Some("text".into()),
                payload: Value::Null,
            }),
        );

        let encoded = match serde_json::to_value(&envelope) {
            Ok(value) => value,
            Err(error) => panic!("serialize envelope: {error}"),
        };

        assert_eq!(encoded["protocol"], IKAROS_PROTOCOL_NAME);
        assert_eq!(encoded["version"], json!(IKAROS_PROTOCOL_VERSION));
        assert_eq!(encoded["kind"], "session_event");
        assert_eq!(encoded["data"]["type"], "assistant_delta");
        assert_eq!(encoded["data"]["data"]["delta"], "hello");
        assert_eq!(encoded["data"]["data"]["part_kind"], "text");

        let decoded: WireEnvelope<SessionEvent> = match serde_json::from_value(encoded) {
            Ok(value) => value,
            Err(error) => panic!("deserialize envelope: {error}"),
        };

        match decoded.data {
            SessionEvent::AssistantDelta(delta) => {
                assert_eq!(delta.session_id, "session-1");
                assert_eq!(delta.delta, "hello");
            }
            other => panic!("expected assistant delta, got {other:?}"),
        }
    }

    #[test]
    fn tool_activity_serializes_children() {
        let activity = ToolActivity {
            id: Some("tool-activity-1".into()),
            title: "Run command".into(),
            kind: "command".into(),
            status: "running".into(),
            children: vec![
                ToolActivityLine {
                    text: "cargo test --lib".into(),
                    kind: Some("command_line".into()),
                    status: None,
                    payload: Value::Null,
                },
                ToolActivityLine {
                    text: "waiting for output".into(),
                    kind: Some("stdout".into()),
                    status: Some("pending".into()),
                    payload: json!({ "stream": "stdout" }),
                },
            ],
            raw: Some(json!({ "provider": "codex" })),
            debug: None,
        };

        let encoded = match serde_json::to_value(&activity) {
            Ok(value) => value,
            Err(error) => panic!("serialize activity: {error}"),
        };

        assert_eq!(encoded["title"], "Run command");
        assert_eq!(encoded["children"][0]["text"], "cargo test --lib");
        assert_eq!(encoded["children"][1]["status"], "pending");
        assert_eq!(encoded["raw"]["provider"], "codex");
        assert!(encoded.get("debug").is_none());

        let decoded: ToolActivity = match serde_json::from_value(encoded) {
            Ok(value) => value,
            Err(error) => panic!("deserialize activity: {error}"),
        };
        assert_eq!(decoded.children.len(), 2);
        assert_eq!(decoded.children[1].payload["stream"], "stdout");
    }

    #[test]
    fn conversation_projection_roundtrips() {
        let projection = ConversationProjection {
            session_id: "session-1".into(),
            title: Some("M6 protocol".into()),
            status: Some("running".into()),
            active_turn_id: Some("turn-1".into()),
            turns: vec![ConversationTurnProjection {
                id: "turn-1".into(),
                parent_turn_id: None,
                status: "running_tool".into(),
                started_at: Some("2026-06-28T00:00:00Z".into()),
                completed_at: None,
                error: None,
                message_ids: vec!["message-1".into(), "message-2".into()],
                tool_ids: vec!["tool-1".into()],
                approval_ids: vec!["approval-1".into()],
                metadata: Value::Null,
            }],
            messages: vec![
                ConversationMessageProjection {
                    id: "message-1".into(),
                    turn_id: Some("turn-1".into()),
                    role: "user".into(),
                    status: Some("submitted".into()),
                    parts: vec![ConversationPartProjection {
                        id: Some("part-1".into()),
                        kind: "text".into(),
                        text: Some("ship it".into()),
                        tool_id: None,
                        approval_id: None,
                        data: Value::Null,
                    }],
                    metadata: Value::Null,
                },
                ConversationMessageProjection {
                    id: "message-2".into(),
                    turn_id: Some("turn-1".into()),
                    role: "assistant".into(),
                    status: Some("streaming".into()),
                    parts: vec![ConversationPartProjection {
                        id: Some("part-2".into()),
                        kind: "tool_activity".into(),
                        text: None,
                        tool_id: Some("tool-1".into()),
                        approval_id: Some("approval-1".into()),
                        data: json!({ "activity_id": "activity-1" }),
                    }],
                    metadata: Value::Null,
                },
            ],
            tools: vec![ConversationToolProjection {
                id: "tool-1".into(),
                turn_id: Some("turn-1".into()),
                call_id: Some("call-1".into()),
                name: "shell".into(),
                status: "running".into(),
                activity: Some(ToolActivity {
                    id: Some("activity-1".into()),
                    title: "Run command".into(),
                    kind: "command".into(),
                    status: "running".into(),
                    children: vec![ToolActivityLine {
                        text: "echo ok".into(),
                        kind: Some("command_line".into()),
                        status: None,
                        payload: Value::Null,
                    }],
                    raw: None,
                    debug: Some(json!({ "pid": "1234" })),
                }),
                input: json!({ "cmd": "echo ok" }),
                output: Value::Null,
                error: None,
                started_at: Some("2026-06-28T00:00:01Z".into()),
                completed_at: None,
                metadata: Value::Null,
            }],
            approvals: vec![ConversationApprovalProjection {
                id: "approval-1".into(),
                turn_id: Some("turn-1".into()),
                tool_id: Some("tool-1".into()),
                title: "Approve shell command".into(),
                status: "requested".into(),
                request: json!({ "cmd": "echo ok" }),
                decision: None,
                response: Value::Null,
                requested_at: Some("2026-06-28T00:00:02Z".into()),
                resolved_at: None,
                metadata: Value::Null,
            }],
            metadata: json!({ "source": "unit_test" }),
        };

        let encoded = match serde_json::to_string(&projection) {
            Ok(value) => value,
            Err(error) => panic!("serialize projection: {error}"),
        };
        let decoded: ConversationProjection = match serde_json::from_str(&encoded) {
            Ok(value) => value,
            Err(error) => panic!("deserialize projection: {error}"),
        };

        assert_eq!(decoded, projection);
    }
}
