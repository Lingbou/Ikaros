// SPDX-License-Identifier: GPL-3.0-only

use super::{
    ChatTurnEventOptions,
    context_prepare::PreparedChatContext,
    events::{emit_chat_event, emit_chat_failure_events},
    session_entries::{
        ChatAssistantEntryInput, ChatSessionEntryStats, append_chat_assistant_session_entry,
    },
    setup::ChatTurnSetup,
};
use crate::{AgentEventSink, record_emotion_signal_with_correlation};
use ikaros_core::{ResolvedAgentProfile, Result, redact_secrets};
use ikaros_harness::{AuditEvent, ExecutionSession, SkillRegistry};
use ikaros_models::{ModelRequestDiagnostic, ModelResponse};
use ikaros_session::{AgentEvent, AgentEventKind, AgentEventSource};
use ikaros_soul::RuntimeSignal;
use serde_json::json;

use crate::chat::{
    context_engine::TurnRecord,
    learning::create_relationship_candidates_from_chat,
    types::{ChatRunOptions, ChatTurnReport},
};

pub(super) struct ChatModelResult {
    pub(super) response: ModelResponse,
    pub(super) streamed: bool,
    pub(super) stream_chunks: Vec<String>,
}

pub(super) struct CompleteChatTurnInput<'a, 'b> {
    pub(super) input: &'a str,
    pub(super) agent: &'a ResolvedAgentProfile,
    pub(super) session: &'a ExecutionSession,
    pub(super) registry: &'a SkillRegistry,
    pub(super) options: &'a ChatRunOptions,
    pub(super) event_options: &'a ChatTurnEventOptions<'b>,
    pub(super) setup: ChatTurnSetup,
    pub(super) prepared: PreparedChatContext<'a>,
    pub(super) model_result: ChatModelResult,
}

pub(super) async fn complete_chat_turn(
    mut input: CompleteChatTurnInput<'_, '_>,
) -> Result<ChatTurnReport> {
    let mut events = std::mem::take(&mut input.setup.single_call_events);
    audit_model_result(
        &mut events,
        input.event_options.event_sink,
        input.session,
        input.options,
        &input.setup,
        &input.model_result,
    )?;
    let final_emotion = record_final_emotion_signal(
        &mut events,
        input.event_options.event_sink,
        input.session,
        &input.setup,
        &input.model_result,
    )?;
    let relationship_candidates_created = create_relationship_candidates(
        &mut events,
        input.event_options.event_sink,
        input.input,
        input.session,
        input.registry,
        input.options,
        &input.setup,
    )
    .await?;
    after_context_turn(
        &mut events,
        input.event_options.event_sink,
        input.input,
        &input.setup,
        &input.prepared,
        &input.model_result.response,
    )
    .await?;
    append_assistant_entry(
        &mut events,
        input.event_options.event_sink,
        input.event_options,
        input.agent,
        &input.setup,
        &input.prepared,
        &input.model_result,
    )?;
    emit_turn_completed(
        &mut events,
        input.event_options.event_sink,
        &input.setup,
        &input.prepared,
        &input.model_result,
    )?;
    Ok(ChatTurnReport {
        response: input.model_result.response,
        emotion: final_emotion,
        streamed: input.model_result.streamed,
        stream_chunks: input.model_result.stream_chunks,
        relationship_hits: input.prepared.chat_context.relationship.len(),
        relationship_candidates_created,
        reference_hits: input.prepared.chat_context.references.len(),
        history_hits: input.prepared.chat_context.history.len(),
        memory_hits: input.prepared.chat_context.memory_hits(),
        rag_hits: input.prepared.chat_context.rag.len(),
        chat_session_id: Some(input.setup.chat_session_id),
    })
}

fn audit_model_result(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    session: &ExecutionSession,
    options: &ChatRunOptions,
    setup: &ChatTurnSetup,
    result: &ChatModelResult,
) -> Result<()> {
    if let Err(error) = session.audit.append(
        AuditEvent::new(
            "chat_model_result",
            None,
            "chat model response generated",
            json!({
                "correlation_id": &setup.correlation_id,
                "provider": &result.response.provider,
                "model": &result.response.model,
                "streamed": result.streamed,
                "agent_loop": setup.effective_agent_loop,
                "agent_loop_requested": options.agent_loop,
                "agent_loop_disabled_reason": if options.agent_loop && setup.content_block_count > 0 {
                    Some("multimodal_content_blocks")
                } else {
                    None
                },
                "content_block_count": setup.content_block_count,
                "chunk_count": result.stream_chunks.len(),
                "usage": &result.response.usage,
                "diagnostics": result.response
                    .diagnostics
                    .iter()
                    .cloned()
                    .map(ModelRequestDiagnostic::sanitized)
                    .collect::<Vec<_>>(),
            }),
        )?
        .with_correlation_id(&setup.correlation_id),
    ) {
        let _ = emit_chat_failure_events(
            events,
            event_sink,
            &setup.session_id,
            &setup.turn_id,
            "audit_chat_model_result",
            &error,
        );
        return Err(error);
    }
    Ok(())
}

fn record_final_emotion_signal(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    session: &ExecutionSession,
    setup: &ChatTurnSetup,
    result: &ChatModelResult,
) -> Result<ikaros_soul::EmotionState> {
    match record_emotion_signal_with_correlation(
        &session.audit,
        RuntimeSignal::TaskComplete,
        "chat response generated",
        json!({
            "provider": &result.response.provider,
            "model": &result.response.model,
            "streamed": result.streamed,
        }),
        Some(&setup.correlation_id),
    ) {
        Ok(emotion) => Ok(emotion),
        Err(error) => {
            let _ = emit_chat_failure_events(
                events,
                event_sink,
                &setup.session_id,
                &setup.turn_id,
                "emotion_signal",
                &error,
            );
            Err(error)
        }
    }
}

async fn create_relationship_candidates(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    input: &str,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
    setup: &ChatTurnSetup,
) -> Result<usize> {
    match create_relationship_candidates_from_chat(
        input,
        &setup.chat_session_id,
        setup.turn_id.as_str(),
        session,
        registry,
        options,
    )
    .await
    {
        Ok(count) => Ok(count),
        Err(error) => {
            let _ = emit_chat_failure_events(
                events,
                event_sink,
                &setup.session_id,
                &setup.turn_id,
                "relationship_candidate_creation",
                &error,
            );
            Err(error)
        }
    }
}

async fn after_context_turn(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    input: &str,
    setup: &ChatTurnSetup,
    prepared: &PreparedChatContext<'_>,
    response: &ModelResponse,
) -> Result<()> {
    if let Err(error) = prepared
        .engine
        .after_turn(TurnRecord {
            session_id: Some(setup.chat_session_id.clone()),
            user_input: redact_secrets(input),
            assistant_output: response.content.clone(),
        })
        .await
    {
        let _ = emit_chat_failure_events(
            events,
            event_sink,
            &setup.session_id,
            &setup.turn_id,
            "context_after_turn",
            &error,
        );
        return Err(error);
    }
    Ok(())
}

fn append_assistant_entry(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    event_options: &ChatTurnEventOptions<'_>,
    agent: &ResolvedAgentProfile,
    setup: &ChatTurnSetup,
    prepared: &PreparedChatContext<'_>,
    result: &ChatModelResult,
) -> Result<()> {
    if let Err(error) = append_chat_assistant_session_entry(ChatAssistantEntryInput {
        session_sink: event_options.session_sink,
        session_id: &setup.session_id,
        turn_id: &setup.turn_id,
        user_entry_id: prepared.assistant_parent_entry_id.clone(),
        agent: &agent.name,
        response: &result.response,
        streamed: result.streamed,
        stats: ChatSessionEntryStats {
            relationship_hits: prepared.chat_context.relationship.len(),
            reference_hits: prepared.chat_context.references.len(),
            memory_hits: prepared.chat_context.memory_hits(),
            rag_hits: prepared.chat_context.rag.len(),
        },
    }) {
        let _ = emit_chat_failure_events(
            events,
            event_sink,
            &setup.session_id,
            &setup.turn_id,
            "session_entry_append",
            &error,
        );
        return Err(error);
    }
    Ok(())
}

fn emit_turn_completed(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    setup: &ChatTurnSetup,
    prepared: &PreparedChatContext<'_>,
    result: &ChatModelResult,
) -> Result<()> {
    if setup.effective_agent_loop {
        return Ok(());
    }
    emit_chat_event(
        events,
        event_sink,
        &setup.session_id,
        &setup.turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "status": "completed",
            "provider": &result.response.provider,
            "model": &result.response.model,
            "streamed": result.streamed,
            "content_block_count": setup.content_block_count,
            "relationship_hits": prepared.chat_context.relationship.len(),
            "reference_hits": prepared.chat_context.references.len(),
            "memory_hits": prepared.chat_context.memory_hits(),
            "rag_hits": prepared.chat_context.rag.len(),
        }),
    )
}
