// SPDX-License-Identifier: GPL-3.0-only

use super::{
    ChatTurnEventOptions,
    events::{emit_chat_event, emit_chat_failure_events},
    model::{redacted_chat_error, validate_content_blocks_supported},
    session_entries::append_chat_user_session_entry,
};
use crate::AgentEventSink;
use ikaros_core::{ResolvedAgentProfile, Result, redact_secrets};
use ikaros_harness::ExecutionSession;
use ikaros_models::{ModelProvider, ModelRequestOptions};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSource, SessionEntryId, SessionId, TurnId,
};
use serde_json::json;

pub(super) struct ChatTurnSetup {
    pub(super) request_options: ModelRequestOptions,
    pub(super) content_block_count: usize,
    pub(super) effective_agent_loop: bool,
    pub(super) chat_session_id: String,
    pub(super) session_id: SessionId,
    pub(super) turn_id: TurnId,
    pub(super) correlation_id: String,
    pub(super) session: ExecutionSession,
    pub(super) single_call_events: Vec<AgentEvent>,
    pub(super) user_entry_id: Option<SessionEntryId>,
}

pub(super) fn setup_chat_turn(
    input: &str,
    provider: &dyn ModelProvider,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    event_options: &ChatTurnEventOptions<'_>,
) -> Result<ChatTurnSetup> {
    let options = event_options.options;
    let request_options = event_options.request_options.cloned().unwrap_or_default();
    let content_block_count = options.content_blocks.len();
    let effective_agent_loop = options.agent_loop && content_block_count == 0;
    let chat_session_id = options
        .session_id
        .clone()
        .unwrap_or_else(super::new_chat_session_id);
    let session_id = SessionId::from(chat_session_id.clone());
    let turn_id = event_options.turn_id.clone().unwrap_or_default();
    let correlation_id = format!("session:{}:turn:{}", session_id.as_str(), turn_id.as_str());
    let session = session.clone().with_correlation_id(correlation_id.clone());
    let mut single_call_events = Vec::new();
    let user_entry_id = append_chat_user_session_entry(
        event_options.session_sink,
        &session_id,
        &turn_id,
        event_options.parent_entry_id.clone(),
        &agent.name,
        input,
        content_block_count,
    )?;
    if !effective_agent_loop {
        emit_single_call_start_events(
            &mut single_call_events,
            event_options.event_sink,
            &session_id,
            &turn_id,
            &correlation_id,
            agent,
            input,
            options.stream,
            options.agent_loop,
            content_block_count,
        )?;
        if let Err(error) = validate_content_blocks_supported(provider, &options.content_blocks) {
            let error = redacted_chat_error(error);
            emit_chat_failure_events(
                &mut single_call_events,
                event_options.event_sink,
                &session_id,
                &turn_id,
                "content_block_preflight",
                &error,
            )?;
            return Err(error);
        }
    }
    Ok(ChatTurnSetup {
        request_options,
        content_block_count,
        effective_agent_loop,
        chat_session_id,
        session_id,
        turn_id,
        correlation_id,
        session,
        single_call_events,
        user_entry_id,
    })
}

#[allow(clippy::too_many_arguments)]
fn emit_single_call_start_events(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    correlation_id: &str,
    agent: &ResolvedAgentProfile,
    input: &str,
    stream: bool,
    agent_loop_requested: bool,
    content_block_count: usize,
) -> Result<()> {
    emit_chat_event(
        events,
        event_sink,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::SessionStart,
        json!({
            "correlation_id": correlation_id,
            "agent": &agent.name,
        }),
    )?;
    emit_chat_event(
        events,
        event_sink,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnStart,
        json!({
            "correlation_id": correlation_id,
            "agent": &agent.name,
            "stream": stream,
            "agent_loop": false,
            "agent_loop_requested": agent_loop_requested,
            "agent_loop_disabled_reason": if agent_loop_requested && content_block_count > 0 {
                Some("multimodal_content_blocks")
            } else {
                None
            },
            "content_block_count": content_block_count,
        }),
    )?;
    emit_chat_event(
        events,
        event_sink,
        session_id,
        turn_id,
        AgentEventSource::User,
        AgentEventKind::UserMessage,
        json!({
            "content": redact_secrets(input),
            "content_block_count": content_block_count,
        }),
    )
}
