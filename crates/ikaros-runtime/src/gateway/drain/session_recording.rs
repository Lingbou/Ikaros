// SPDX-License-Identifier: GPL-3.0-only

use super::status::gateway_status_str;
use crate::session::{
    RuntimeSessionEntryInput, RuntimeSessionTarget, active_leaf_entry_id,
    append_runtime_session_entry, append_runtime_session_event, delivery_payload,
    gateway_message_kind, gateway_session_id, gateway_session_source, gateway_turn_id,
    runtime_session_target, runtime_session_target_for_evidence, upsert_runtime_session,
};
use ikaros_core::{IkarosPaths, Result, redact_secrets};
use ikaros_gateway::{GatewayDelivery, GatewayMessage, GatewayMessageStatus};
use ikaros_harness::TaskExecutionReport;
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, PersistingAgentTurnSink,
    SessionEntry, SessionEntryId, SessionEntryKind, SessionInputAdmission, SessionStore,
};
use serde_json::json;
use std::{path::Path, sync::Arc};

pub(super) fn record_gateway_delivery_session(
    paths: &IkarosPaths,
    workspace: &Path,
    agent: Option<&str>,
    message: &GatewayMessage,
    status: GatewayMessageStatus,
    summary: &str,
    delivery: Option<&GatewayDelivery>,
) -> Result<()> {
    let target = gateway_error_session_target(paths, workspace, agent, message)?;
    let session_id = gateway_session_id(message);
    upsert_runtime_session(&target, &session_id, gateway_session_source(message))?;
    let turn_id = gateway_turn_id(&message.id);
    if status != GatewayMessageStatus::Processed
        && !gateway_turn_has_events(&target, &session_id, &turn_id)?
    {
        append_gateway_error_timeline(
            &target,
            &session_id,
            &turn_id,
            message,
            status.clone(),
            summary,
        )?;
    }
    let parent_entry_id = active_leaf_entry_id(&target, &session_id)?;
    let delivery_value = delivery.map(|delivery| {
        json!({
            "delivery_id": &delivery.id,
            "kind": &delivery.kind,
            "message_id": &delivery.message_id,
            "created_at": &delivery.created_at,
        })
    });
    append_runtime_session_entry(RuntimeSessionEntryInput {
        target: &target,
        session_id: &session_id,
        parent_entry_id,
        turn_id: &turn_id,
        kind: SessionEntryKind::Custom,
        visible_text: Some(redact_secrets(summary)),
        payload: delivery_payload(
            "gateway_delivery",
            gateway_status_str(&status),
            summary,
            delivery_value,
        ),
    })?;
    Ok(())
}

fn gateway_error_session_target(
    paths: &IkarosPaths,
    workspace: &Path,
    agent: Option<&str>,
    message: &GatewayMessage,
) -> Result<RuntimeSessionTarget> {
    runtime_session_target_for_evidence(
        paths,
        workspace,
        agent,
        &fallback_gateway_agent_id(agent, message),
    )
}

fn fallback_gateway_agent_id(agent: Option<&str>, message: &GatewayMessage) -> String {
    agent
        .or(message.agent.as_deref())
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("build")
        .to_owned()
}

fn gateway_turn_has_events(
    target: &RuntimeSessionTarget,
    session_id: &ikaros_session::SessionId,
    turn_id: &ikaros_session::TurnId,
) -> Result<bool> {
    Ok(target
        .store
        .replay_session(session_id)?
        .is_some_and(|replay| {
            replay
                .agent_events
                .iter()
                .any(|event| event.turn_id == *turn_id)
        }))
}

fn append_gateway_error_timeline(
    target: &RuntimeSessionTarget,
    session_id: &ikaros_session::SessionId,
    turn_id: &ikaros_session::TurnId,
    message: &GatewayMessage,
    status: GatewayMessageStatus,
    summary: &str,
) -> Result<()> {
    append_runtime_session_event(
        target,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::SessionStart,
        json!({
            "source": "gateway",
            "message_id": &message.id,
            "kind": gateway_message_kind(&message.kind),
        }),
    )?;
    append_runtime_session_event(
        target,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnStart,
        json!({
            "source": "gateway",
            "message_id": &message.id,
            "kind": gateway_message_kind(&message.kind),
        }),
    )?;
    let user_entry_id = append_runtime_session_entry(RuntimeSessionEntryInput {
        target,
        session_id,
        parent_entry_id: active_leaf_entry_id(target, session_id)?,
        turn_id,
        kind: SessionEntryKind::UserMessage,
        visible_text: Some(redact_secrets(&message.content)),
        payload: json!({
            "role": "user",
            "source": "gateway",
            "message_id": &message.id,
            "kind": gateway_message_kind(&message.kind),
            "content": redact_secrets(&message.content),
        }),
    })?;
    append_runtime_session_event(
        target,
        session_id,
        turn_id,
        AgentEventSource::User,
        AgentEventKind::UserMessage,
        json!({
            "content": redact_secrets(&message.content),
        }),
    )?;
    append_runtime_session_entry(RuntimeSessionEntryInput {
        target,
        session_id,
        parent_entry_id: Some(user_entry_id),
        turn_id,
        kind: SessionEntryKind::Custom,
        visible_text: Some(redact_secrets(summary)),
        payload: delivery_payload("gateway_error", gateway_status_str(&status), summary, None),
    })?;
    append_runtime_session_event(
        target,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::Error,
        json!({
            "source": "gateway",
            "message_id": &message.id,
            "summary": redact_secrets(summary),
        }),
    )?;
    append_runtime_session_event(
        target,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "source": "gateway",
            "message_id": &message.id,
            "status": gateway_status_str(&status),
        }),
    )
}

pub(super) struct GatewayTaskSessionInput<'a> {
    pub(super) paths: &'a IkarosPaths,
    pub(super) workspace: &'a Path,
    pub(super) agent: Option<&'a str>,
    pub(super) message: &'a GatewayMessage,
    pub(super) status: GatewayMessageStatus,
    pub(super) summary: &'a str,
    pub(super) delivery: Option<&'a GatewayDelivery>,
    pub(super) task_report: Option<&'a TaskExecutionReport>,
}

pub(super) fn record_gateway_task_session(input: GatewayTaskSessionInput<'_>) -> Result<()> {
    let target = runtime_session_target(input.paths, input.workspace, input.agent)?;
    let session_id = gateway_session_id(input.message);
    let turn_id = gateway_turn_id(&input.message.id);
    let admitted_input = target.store.admit_input(&SessionInputAdmission::new(
        session_id.clone(),
        gateway_input_payload(input.message),
    ))?;
    let turn_sink = PersistingAgentTurnSink::new(Arc::new(target.store.clone()))
        .with_source(gateway_session_source(input.message))
        .with_agent_id(target.agent_id.clone())
        .with_workspace(target.workspace.clone());
    turn_sink.promote_input_on_commit(admitted_input.input_id.clone())?;
    let parent_entry_id = active_leaf_entry_id(&target, &session_id)?;
    let result = (|| -> Result<()> {
        append_gateway_buffered_event(
            &turn_sink,
            &session_id,
            &turn_id,
            AgentEventSource::Runtime,
            AgentEventKind::SessionStart,
            json!({
                "source": "gateway",
                "message_id": &input.message.id,
                "kind": gateway_message_kind(&input.message.kind),
            }),
        )?;
        append_gateway_buffered_event(
            &turn_sink,
            &session_id,
            &turn_id,
            AgentEventSource::Runtime,
            AgentEventKind::TurnStart,
            json!({
                "source": "gateway",
                "message_id": &input.message.id,
                "kind": gateway_message_kind(&input.message.kind),
            }),
        )?;
        let user_entry_id = append_gateway_buffered_entry(GatewayBufferedEntryInput {
            sink: &turn_sink,
            session_id: &session_id,
            parent_entry_id,
            turn_id: &turn_id,
            kind: SessionEntryKind::UserMessage,
            visible_text: Some(redact_secrets(&input.message.content)),
            payload: gateway_input_payload(input.message),
        })?;
        append_gateway_buffered_event(
            &turn_sink,
            &session_id,
            &turn_id,
            AgentEventSource::User,
            AgentEventKind::UserMessage,
            json!({
                "content": redact_secrets(&input.message.content),
            }),
        )?;
        let delivery = input.delivery.map(|delivery| {
            json!({
                "delivery_id": &delivery.id,
                "kind": &delivery.kind,
                "message_id": &delivery.message_id,
                "created_at": &delivery.created_at,
            })
        });
        append_gateway_buffered_entry(GatewayBufferedEntryInput {
            sink: &turn_sink,
            session_id: &session_id,
            parent_entry_id: Some(user_entry_id),
            turn_id: &turn_id,
            kind: SessionEntryKind::Custom,
            visible_text: Some(redact_secrets(input.summary)),
            payload: json!({
                "role": "runtime",
                "source": "gateway",
                "message_id": &input.message.id,
                "status": gateway_status_str(&input.status),
                "summary": redact_secrets(input.summary),
                "task_id": input.task_report.map(|report| report.task_id.as_str()),
                "task_state": input.task_report.map(|report| format!("{:?}", report.state)),
                "step_count": input.task_report.map(|report| report.steps.len()),
                "delivery": delivery.unwrap_or(serde_json::Value::Null),
            }),
        })?;
        if matches!(
            input.status,
            GatewayMessageStatus::Failed | GatewayMessageStatus::DeadLettered
        ) {
            append_gateway_buffered_event(
                &turn_sink,
                &session_id,
                &turn_id,
                AgentEventSource::Runtime,
                AgentEventKind::Error,
                json!({
                    "source": "gateway",
                    "message_id": &input.message.id,
                    "summary": redact_secrets(input.summary),
                }),
            )?;
        }
        append_gateway_buffered_event(
            &turn_sink,
            &session_id,
            &turn_id,
            AgentEventSource::Runtime,
            AgentEventKind::TurnEnd,
            json!({
                "source": "gateway",
                "message_id": &input.message.id,
                "status": gateway_status_str(&input.status),
            }),
        )?;
        turn_sink.commit()
    })();
    if result.is_err() {
        let _ = turn_sink.rollback();
        let _ = target
            .store
            .cancel_input(&admitted_input.input_id, "gateway_task_session_failed");
    }
    result
}

fn gateway_input_payload(message: &GatewayMessage) -> serde_json::Value {
    json!({
        "role": "user",
        "source": "gateway",
        "message_id": &message.id,
        "kind": gateway_message_kind(&message.kind),
        "content": redact_secrets(&message.content),
    })
}

struct GatewayBufferedEntryInput<'a> {
    sink: &'a PersistingAgentTurnSink,
    session_id: &'a ikaros_session::SessionId,
    parent_entry_id: Option<SessionEntryId>,
    turn_id: &'a ikaros_session::TurnId,
    kind: SessionEntryKind,
    visible_text: Option<String>,
    payload: serde_json::Value,
}

fn append_gateway_buffered_entry(input: GatewayBufferedEntryInput<'_>) -> Result<SessionEntryId> {
    let mut entry = SessionEntry::new(input.session_id.clone(), input.kind);
    entry.parent_entry_id = input.parent_entry_id;
    entry.turn_id = Some(input.turn_id.clone());
    entry.visible_text = input.visible_text;
    entry.payload = input.payload;
    let entry_id = entry.entry_id.clone();
    input.sink.append_entry(&entry)?;
    Ok(entry_id)
}

fn append_gateway_buffered_event(
    sink: &PersistingAgentTurnSink,
    session_id: &ikaros_session::SessionId,
    turn_id: &ikaros_session::TurnId,
    source: AgentEventSource,
    kind: AgentEventKind,
    payload: serde_json::Value,
) -> Result<()> {
    sink.emit(&AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        source,
        kind,
        payload,
    ))
}
