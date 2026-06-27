// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{EmotionState, IkarosPaths, PolicyDecision, Result, load_or_default};
use ikaros_execution::harness::{AuditEvent, AuditLog};
use ikaros_protocol::{BodyEvent, BodyEventKind, BodyFrame, BodyKind, BodyStatus};
use serde_json::Value;
use std::collections::BTreeMap;

const EMOTION_EVENT_KIND: &str = "emotion_state";

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
    let persona = load_or_default(&paths.persona_dir)?;
    let emotion = latest_body_emotion_from_events(events);
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

fn latest_body_emotion_from_events(events: &[AuditEvent]) -> EmotionState {
    events
        .iter()
        .rev()
        .find(|event| event.kind == EMOTION_EVENT_KIND)
        .and_then(|event| event.data.get("emotion"))
        .and_then(Value::as_str)
        .and_then(parse_body_emotion_state)
        .unwrap_or(EmotionState::Neutral)
}

fn parse_body_emotion_state(value: &str) -> Option<EmotionState> {
    match value {
        "Neutral" => Some(EmotionState::Neutral),
        "Focused" => Some(EmotionState::Focused),
        "Curious" => Some(EmotionState::Curious),
        "Confused" => Some(EmotionState::Confused),
        "Concerned" => Some(EmotionState::Concerned),
        "Satisfied" => Some(EmotionState::Satisfied),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn latest_body_emotion_defaults_to_neutral_and_reads_latest_event() {
        let focused = AuditEvent::new(
            EMOTION_EVENT_KIND,
            None,
            "focused",
            json!({"emotion": "Focused"}),
        )
        .expect("focused");
        let concerned = AuditEvent::new(
            EMOTION_EVENT_KIND,
            None,
            "concerned",
            json!({"emotion": "Concerned"}),
        )
        .expect("concerned");

        assert_eq!(latest_body_emotion_from_events(&[]), EmotionState::Neutral);
        assert_eq!(
            latest_body_emotion_from_events(&[focused, concerned]),
            EmotionState::Concerned
        );
    }
}
