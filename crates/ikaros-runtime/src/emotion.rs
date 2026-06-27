// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::Result;
use ikaros_harness::{AuditEvent, AuditLog};
use ikaros_soul::{EmotionState, RuntimeSignal};
use serde_json::{Map, Value, json};

pub const EMOTION_EVENT_KIND: &str = "emotion_state";

pub fn record_emotion_signal(
    audit: &AuditLog,
    signal: RuntimeSignal,
    reason: impl Into<String>,
    data: Value,
) -> Result<EmotionState> {
    record_emotion_signal_with_correlation(audit, signal, reason, data, None)
}

pub fn record_emotion_signal_with_correlation(
    audit: &AuditLog,
    signal: RuntimeSignal,
    reason: impl Into<String>,
    data: Value,
    correlation_id: Option<&str>,
) -> Result<EmotionState> {
    let emotion = EmotionState::for_runtime_signal(signal);
    let mut payload = match data {
        Value::Object(object) => object,
        Value::Null => Map::new(),
        other => {
            let mut object = Map::new();
            object.insert("details".into(), other);
            object
        }
    };
    payload.insert("signal".into(), json!(format!("{signal:?}")));
    payload.insert("emotion".into(), json!(format!("{emotion:?}")));
    payload.insert("reason".into(), json!(reason.into()));
    if let Some(correlation_id) = correlation_id.filter(|value| !value.trim().is_empty()) {
        payload.insert("correlation_id".into(), json!(correlation_id));
    }
    let mut event = AuditEvent::new(
        EMOTION_EVENT_KIND,
        None,
        format!("emotion state updated: {emotion:?}"),
        Value::Object(payload),
    )?;
    if let Some(correlation_id) = correlation_id {
        event = event.with_correlation_id(correlation_id);
    }
    audit.append(event)?;
    Ok(emotion)
}

pub fn latest_emotion_from_events(events: &[AuditEvent]) -> EmotionState {
    events
        .iter()
        .rev()
        .find(|event| event.kind == EMOTION_EVENT_KIND)
        .and_then(|event| event.data.get("emotion"))
        .and_then(Value::as_str)
        .and_then(parse_emotion_state)
        .unwrap_or(EmotionState::Neutral)
}

pub fn parse_emotion_state(value: &str) -> Option<EmotionState> {
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

    #[test]
    fn records_and_recovers_latest_emotion_event() {
        let temp = tempfile::tempdir().expect("tempdir");
        let audit = AuditLog::new(temp.path());

        record_emotion_signal(
            &audit,
            RuntimeSignal::Planning,
            "task planning",
            json!({"task_id": "task-1"}),
        )
        .expect("planning");
        record_emotion_signal(
            &audit,
            RuntimeSignal::TaskComplete,
            "task completed",
            json!({"task_id": "task-1"}),
        )
        .expect("complete");

        let events = audit.read_all().expect("events");
        assert_eq!(latest_emotion_from_events(&events), EmotionState::Satisfied);
        assert!(events.iter().any(|event| event.kind == EMOTION_EVENT_KIND));
    }
}
