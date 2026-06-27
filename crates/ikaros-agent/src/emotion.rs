// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{EmotionState, Result, RuntimeSignal};
use ikaros_execution::harness::{AuditEvent, AuditLog};
use serde_json::{Map, Value, json};

pub(crate) const EMOTION_EVENT_KIND: &str = "emotion_state";

pub(crate) fn record_emotion_signal(
    audit: &AuditLog,
    signal: RuntimeSignal,
    reason: impl Into<String>,
    data: Value,
) -> Result<EmotionState> {
    record_emotion_signal_with_correlation(audit, signal, reason, data, None)
}

pub(crate) fn record_emotion_signal_with_correlation(
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
