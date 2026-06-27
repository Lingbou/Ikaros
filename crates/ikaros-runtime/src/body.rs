// SPDX-License-Identifier: GPL-3.0-only

use crate::latest_emotion_from_events;
use ikaros_body::{BodyEvent, BodyEventKind, BodyFrame, BodyKind, BodyStatus};
use ikaros_core::{IkarosPaths, PolicyDecision, Result};
use ikaros_harness::{AuditEvent, AuditLog};
use ikaros_soul::load_or_default;
use serde_json::Value;
use std::collections::BTreeMap;

pub fn current_body_frame(
    paths: &IkarosPaths,
    event_limit: usize,
    body: BodyKind,
) -> Result<BodyFrame> {
    let audit = AuditLog::new(&paths.audit_dir);
    let events = audit.read_all()?;
    let mut status = base_body_status_from_events(paths, &events)?;
    status = status.with_policy_decisions(recent_policy_decisions_from_events(&events));
    Ok(BodyFrame {
        body: body.clone(),
        status,
        events: recent_body_events(events, event_limit, body),
    })
}

pub fn base_body_status(paths: &IkarosPaths) -> Result<BodyStatus> {
    let audit = AuditLog::new(&paths.audit_dir);
    let events = audit.read_all()?;
    base_body_status_from_events(paths, &events)
}

fn base_body_status_from_events(paths: &IkarosPaths, events: &[AuditEvent]) -> Result<BodyStatus> {
    let persona = load_or_default(&paths.persona)?;
    let emotion = latest_emotion_from_events(events);
    Ok(
        BodyStatus::new(persona.identity.name, format!("{emotion:?}"))
            .with_audit_path(paths.audit_dir.join("audit.jsonl"))
            .with_approvals_path(paths.audit_dir.join("approvals.jsonl")),
    )
}

fn recent_body_events(events: Vec<AuditEvent>, limit: usize, body: BodyKind) -> Vec<BodyEvent> {
    let start = events.len().saturating_sub(limit);
    events
        .into_iter()
        .skip(start)
        .map(|event| audit_event_to_body_event_for_body(event, body.clone()))
        .collect()
}

fn recent_policy_decisions_from_events(events: &[AuditEvent]) -> Vec<PolicyDecision> {
    let decisions = events
        .iter()
        .filter_map(|event| event.decision.clone())
        .collect::<Vec<_>>();
    let start = decisions.len().saturating_sub(12);
    decisions[start..].to_vec()
}

pub fn audit_event_to_body_event(event: AuditEvent) -> BodyEvent {
    audit_event_to_body_event_for_body(event, BodyKind::Web)
}

pub fn audit_event_to_body_event_for_body(event: AuditEvent, body: BodyKind) -> BodyEvent {
    let mut data: BTreeMap<String, Value> = BTreeMap::new();
    data.insert("at".into(), Value::String(event.at));
    data.insert("kind".into(), Value::String(event.kind.clone()));
    if let Some(decision) = event.decision {
        data.insert("decision".into(), Value::String(format!("{decision:?}")));
    }
    if let Some(object) = event.data.as_object() {
        let keys = object.keys().cloned().collect::<Vec<_>>().join(",");
        if !keys.is_empty() {
            data.insert("data_keys".into(), Value::String(keys));
        }
        for key in ["call_id", "approval_id", "task_id", "emotion", "signal"] {
            if let Some(value) = object.get(key).and_then(serde_json::Value::as_str) {
                data.insert(key.into(), Value::String(value.into()));
            }
        }
    }
    data.insert("audit_data".into(), event.data);
    BodyEvent::new(
        body,
        body_event_kind_from_audit(&event.kind),
        event.message,
        data,
    )
}

pub fn body_event_kind_from_audit(kind: &str) -> BodyEventKind {
    match kind {
        "emotion_state" => BodyEventKind::Emotion,
        "policy_decision" | "approval_decision" | "approval_executed" => BodyEventKind::Approval,
        "task_execution_start"
        | "task_step_start"
        | "task_step_result"
        | "task_execution_end"
        | "task_step_retry" => BodyEventKind::Task,
        "tool_call" | "tool_result" => BodyEventKind::Skill,
        "chat_context_built" | "chat_model_result" | "code_model_review_result" => {
            BodyEventKind::Message
        }
        _ => BodyEventKind::Audit,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_core::PolicyDecision;
    use ikaros_soul::RuntimeSignal;
    use serde_json::json;

    #[test]
    fn audit_event_mapping_redacts_data_values() {
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
            body_event
                .data
                .get("call_id")
                .and_then(serde_json::Value::as_str),
            Some("call-1")
        );
        assert!(!body_event.message.contains("abc123"));
        assert!(
            !body_event
                .data
                .values()
                .any(|value| value.to_string().contains("abc123"))
        );
    }

    #[test]
    fn audit_event_mapping_preserves_typed_json_data() {
        let event = AuditEvent::new(
            "task_step_result",
            None,
            "task event",
            json!({
                "task_id": "job-1",
                "attempt": 2,
                "ok": false,
                "details": {
                    "file": "src/main.rs",
                    "token": "api_key=abc123"
                }
            }),
        )
        .expect("event");
        let body_event = audit_event_to_body_event(event);

        assert_eq!(body_event.kind, BodyEventKind::Task);
        assert_eq!(body_event.data["task_id"], "job-1");
        assert_eq!(body_event.data["audit_data"]["attempt"], 2);
        assert_eq!(body_event.data["audit_data"]["ok"], false);
        assert_eq!(
            body_event.data["audit_data"]["details"]["file"],
            "src/main.rs"
        );
        assert_eq!(
            body_event.data["audit_data"]["details"]["token"],
            "[REDACTED_SECRET]"
        );
    }

    #[test]
    fn current_body_frame_reads_recent_audit_events() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path().join("home"));
        paths.ensure().expect("paths");
        AuditLog::new(&paths.audit_dir)
            .append(
                AuditEvent::new(
                    "policy_decision",
                    Some(PolicyDecision::Allow),
                    "allowed safe read",
                    json!({"action": "memory_search"}),
                )
                .expect("event"),
            )
            .expect("audit");

        let frame = current_body_frame(&paths, 5, BodyKind::Web).expect("frame");

        assert_eq!(frame.body, BodyKind::Web);
        assert_eq!(frame.status.persona_name, "Ikaros");
        assert_eq!(frame.status.policy_decisions, vec![PolicyDecision::Allow]);
        assert_eq!(frame.events.len(), 1);
        assert_eq!(frame.events[0].kind, BodyEventKind::Approval);
    }

    #[test]
    fn current_body_frame_uses_latest_emotion_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = IkarosPaths::from_home(temp.path().join("home"));
        paths.ensure().expect("paths");
        crate::record_emotion_signal(
            &AuditLog::new(&paths.audit_dir),
            RuntimeSignal::RiskAction,
            "approval needed",
            json!({"task_id": "task-1"}),
        )
        .expect("emotion");

        let frame = current_body_frame(&paths, 5, BodyKind::Web).expect("frame");

        assert_eq!(frame.status.emotion, "Concerned");
        assert_eq!(frame.events.len(), 1);
        assert_eq!(frame.events[0].kind, BodyEventKind::Emotion);
        assert_eq!(
            frame.events[0]
                .data
                .get("emotion")
                .and_then(serde_json::Value::as_str),
            Some("Concerned")
        );
    }
}
