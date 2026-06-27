// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::redact_secrets;
use ikaros_protocol::{GatewayMessage, GatewayMessageKind};
use ring::digest::{SHA256, digest};
use std::fmt::Write as _;

use super::{SessionId, SessionSource, TurnId};

pub fn gateway_session_id(message: &GatewayMessage) -> SessionId {
    if let Some(source) = &message.session_source {
        return SessionId::from(format!(
            "gateway-{}",
            gateway_session_digest(
                &source.channel,
                source.account.as_deref().unwrap_or("_"),
                source.peer.as_deref().unwrap_or("_"),
                source.thread.as_deref().unwrap_or("_"),
            )
        ));
    }

    SessionId::from(format!(
        "gateway-message-{}",
        gateway_session_digest(&message.source, "_", "_", &message.id)
    ))
}

pub fn gateway_session_source(message: &GatewayMessage) -> SessionSource {
    let source = message.session_source.as_ref();
    SessionSource::Gateway {
        channel: redact_secrets(
            source
                .map(|source| source.channel.as_str())
                .unwrap_or(message.source.as_str()),
        ),
        account: source
            .and_then(|source| source.account.as_deref())
            .map(redact_secrets),
        peer: source
            .and_then(|source| source.peer.as_deref())
            .map(redact_secrets),
        thread: source
            .and_then(|source| source.thread.as_deref())
            .map(redact_secrets),
        message_id: source
            .and_then(|source| source.message_id.as_deref())
            .map(redact_secrets)
            .or_else(|| Some(redact_secrets(&message.id))),
    }
}

pub fn gateway_turn_id(message_id: &str) -> TurnId {
    TurnId::from(format!("gateway-{message_id}"))
}

pub fn gateway_message_kind(kind: &GatewayMessageKind) -> &'static str {
    match kind {
        GatewayMessageKind::Chat => "chat",
        GatewayMessageKind::Task => "task",
    }
}

pub fn schedule_session_id(job_id: &str) -> SessionId {
    SessionId::from(format!("schedule:{}", redact_session_segment(job_id)))
}

pub fn schedule_turn_id(run_id: &str) -> TurnId {
    TurnId::from(format!("schedule-{run_id}"))
}

pub fn schedule_session_source(job_id: &str) -> SessionSource {
    SessionSource::Schedule {
        job_id: redact_secrets(job_id),
    }
}

fn gateway_session_digest(channel: &str, account: &str, peer: &str, thread: &str) -> String {
    let mut input = Vec::new();
    input.extend_from_slice(b"ikaros.gateway.session.v1\0");
    push_digest_part(&mut input, channel);
    push_digest_part(&mut input, account);
    push_digest_part(&mut input, peer);
    push_digest_part(&mut input, thread);
    let digest = digest(&SHA256, &input);
    let mut encoded = String::new();
    for byte in &digest.as_ref()[..12] {
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn push_digest_part(input: &mut Vec<u8>, value: &str) {
    input.extend_from_slice(value.as_bytes());
    input.push(0);
}

fn redact_session_segment(value: &str) -> String {
    let sanitized = redact_secrets(value).replace(['/', '\\', ':', '\n', '\r', '\t'], "_");
    if sanitized.trim().is_empty() {
        "build".into()
    } else {
        sanitized
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::{GatewayRoute, GatewaySessionSource};

    #[test]
    fn gateway_session_id_uses_digest_without_raw_route_identity() {
        let first_source = GatewaySessionSource {
            channel: "telegram".into(),
            account: Some("account-a".into()),
            peer: Some("peer-a".into()),
            thread: Some("thread-a".into()),
            message_id: Some("message-1".into()),
        };
        let second_source = GatewaySessionSource {
            message_id: Some("message-2".into()),
            ..first_source.clone()
        };
        let first = GatewayMessage::new(
            GatewayRoute::new(
                "telegram",
                GatewayMessageKind::Chat,
                "hello",
                Some("build".into()),
            )
            .with_session_source(first_source),
        )
        .expect("first");
        let second = GatewayMessage::new(
            GatewayRoute::new(
                "telegram",
                GatewayMessageKind::Chat,
                "continue",
                Some("build".into()),
            )
            .with_session_source(second_source),
        )
        .expect("second");

        let first_id = gateway_session_id(&first);
        let second_id = gateway_session_id(&second);

        assert_eq!(first_id, second_id);
        assert!(first_id.as_str().starts_with("gateway-"));
        assert!(!first_id.as_str().contains("telegram"));
        assert!(!first_id.as_str().contains("account-a"));
        assert!(!first_id.as_str().contains("peer-a"));
        assert!(!first_id.as_str().contains("thread-a"));
        assert!(!first_id.as_str().contains("message-1"));

        let source = gateway_session_source(&first);
        match source {
            SessionSource::Gateway { message_id, .. } => {
                assert_eq!(message_id.as_deref(), Some("message-1"));
            }
            other => panic!("unexpected source: {other:?}"),
        }
    }

    #[test]
    fn gateway_session_id_changes_for_distinct_gateway_threads() {
        let base = GatewaySessionSource {
            channel: "telegram".into(),
            account: Some("account-a".into()),
            peer: Some("peer-a".into()),
            thread: Some("thread-a".into()),
            message_id: Some("message-1".into()),
        };
        let first = GatewayMessage::new(
            GatewayRoute::new("telegram", GatewayMessageKind::Chat, "hello", None)
                .with_session_source(base.clone()),
        )
        .expect("first");
        let second = GatewayMessage::new(
            GatewayRoute::new("telegram", GatewayMessageKind::Chat, "hello", None)
                .with_session_source(GatewaySessionSource {
                    thread: Some("thread-b".into()),
                    ..base
                }),
        )
        .expect("second");

        assert_ne!(gateway_session_id(&first), gateway_session_id(&second));
    }
}
