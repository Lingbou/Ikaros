// SPDX-License-Identifier: GPL-3.0-only

use super::{
    context::chat_context_char_count,
    context_engine::{
        ContextEngine, ContextEvent, LocalChatContextEngine, TurnRecord,
        build_chat_context_with_engine,
    },
    history::{
        ChatHistoryAppend, ChatHistoryStore, build_chat_history_record_with_turn_id,
        new_chat_session_id,
    },
    learning::learn_relationships_from_chat,
    prompt::{render_chat_system_prompt, render_persona_agent_context},
    types::{ChatMessageResult, ChatRunOptions, ChatTurnReport},
};
use crate::{
    AgentEventSink, AgentLoopInput, AgentLoopOptions, noop_agent_event_sink,
    run_agent_loop_with_events,
};
use crate::{record_emotion_signal, resolve_agent_instance, session_and_registry_for_instance};
use ikaros_core::{
    ContextBuilder, IkarosConfig, IkarosError, IkarosPaths, ResolvedAgentProfile, Result,
    redact_secrets,
};
use ikaros_harness::GuardrailConfig;
use ikaros_harness::{AuditEvent, ExecutionSession, SkillRegistry};
use ikaros_memory::{LocalMemoryStore, MemoryProvider, MemoryTurnRecord, MemoryTurnStart};
use ikaros_models::{
    ModelMessage, ModelProvider, ModelRequest, ModelResponse, ModelStreamEvent, ModelUsageLedger,
    governed_provider_from_config,
};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSource, PersistingAgentTurnSink, SessionEntry,
    SessionEntryId, SessionEntryKind, SessionId, SessionSource, SessionStore, SqliteSessionStore,
    TurnId,
};
use ikaros_soul::{PersonaProfile, RuntimeSignal, load_or_default};
use serde_json::json;
use std::{path::Path, sync::Arc};

pub async fn run_chat_message(
    message: &str,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    options: ChatRunOptions,
) -> Result<ChatMessageResult> {
    paths.ensure()?;
    let config = IkarosConfig::load(&paths.config)?;
    let agent_instance = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let agent = ResolvedAgentProfile {
        name: agent_instance.profile_name.clone(),
        profile: agent_instance.profile.clone(),
    };
    let persona = load_or_default(&paths.persona)?;
    let provider = governed_provider_from_config(
        &config.model.default,
        &config.providers.model,
        &paths.audit_dir,
    )?;
    let (session, registry) = session_and_registry_for_instance(paths, &config, &agent_instance)?;
    let memory_provider = LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)?;
    let mut options = options;
    if options.session_id.is_none() {
        options.session_id = Some(new_chat_session_id());
    }
    let chat_session_id = options
        .session_id
        .clone()
        .expect("chat session id initialized");
    let turn_id = TurnId::new();
    let session_store: Arc<dyn SessionStore> =
        Arc::new(SqliteSessionStore::new(&agent_instance.state_dir));
    let parent_entry_id = session_store
        .get_session(&SessionId::from(chat_session_id.clone()))?
        .and_then(|session| session.active_leaf_entry_id);
    let session_source = options.session_source.clone().unwrap_or(SessionSource::Cli);
    let event_sink = PersistingAgentTurnSink::new(session_store)
        .with_source(session_source)
        .with_agent_id(agent_instance.agent_id.clone())
        .with_workspace(agent_instance.workspace.clone());
    memory_provider.turn_start(MemoryTurnStart {
        session_id: options.session_id.clone(),
        agent_id: Some(agent_instance.agent_id.clone()),
        user_input: redact_secrets(message),
    })?;
    let usage_ledger = ModelUsageLedger::new(&paths.audit_dir);
    let history_store =
        ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    options.chat_history_path = Some(history_store.path().to_path_buf());
    options.chat_history_backend = Some(history_store.backend_name().into());
    let report = match run_chat_turn_with_events(
        message,
        &persona,
        provider.as_ref(),
        &agent,
        &session,
        &registry,
        ChatTurnEventOptions {
            options: &options,
            event_sink: &event_sink,
            session_sink: Some(&event_sink),
            parent_entry_id,
            turn_id: Some(turn_id.clone()),
        },
    )
    .await
    {
        Ok(report) => report,
        Err(error) => {
            if event_sink.commit().is_err() {
                let _ = event_sink.rollback();
            }
            return Err(error);
        }
    };
    if let Err(error) = memory_provider.sync_turn(MemoryTurnRecord {
        session_id: report.chat_session_id.clone(),
        agent_id: Some(agent_instance.agent_id.clone()),
        user_input: redact_secrets(message),
        assistant_output: report.response.content.clone(),
    }) {
        let _ = emit_chat_failure_event(
            &event_sink,
            &SessionId::from(chat_session_id.clone()),
            &turn_id,
            "memory_sync",
            &error,
        );
        if event_sink.commit().is_err() {
            let _ = event_sink.rollback();
        }
        return Err(error);
    }
    emit_chat_lifecycle_event(
        &event_sink,
        &SessionId::from(chat_session_id.clone()),
        &turn_id,
        AgentEventSource::Memory,
        AgentEventKind::MemoryLifecycle,
        json!({
            "phase": "sync_turn",
            "status": "ok",
            "agent_id": &agent_instance.agent_id,
            "session_id": &chat_session_id,
        }),
    )?;
    emit_chat_lifecycle_event(
        &event_sink,
        &SessionId::from(chat_session_id.clone()),
        &turn_id,
        AgentEventSource::Audit,
        AgentEventKind::AuditAnchor,
        json!({
            "audit_path": session.audit.path().display().to_string(),
            "model_usage_path": usage_ledger.path().display().to_string(),
            "chat_history_path": report.chat_history_path.as_ref().map(|path| path.display().to_string()),
            "chat_history_backend": options.chat_history_backend.as_deref(),
        }),
    )?;
    event_sink.commit()?;
    let chat_history_path = report
        .chat_history_path
        .clone()
        .unwrap_or_else(|| history_store.path().to_path_buf());
    let chat_session_id = report.chat_session_id.clone().unwrap_or_else(|| {
        options
            .session_id
            .clone()
            .unwrap_or_else(new_chat_session_id)
    });
    let response = report.response;
    Ok(ChatMessageResult {
        content: response.content,
        provider: response.provider,
        model: response.model,
        emotion: report.emotion,
        streamed: report.streamed,
        stream_chunks: report.stream_chunks,
        relationship_hits: report.relationship_hits,
        relationship_learned: report.relationship_learned,
        history_hits: report.history_hits,
        memory_hits: report.memory_hits,
        rag_hits: report.rag_hits,
        audit_path: session.audit.path().to_path_buf(),
        model_usage_path: usage_ledger.path().to_path_buf(),
        chat_history_path,
        chat_session_id,
    })
}

pub async fn run_chat_turn(
    input: &str,
    persona: &PersonaProfile,
    provider: &dyn ModelProvider,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<ChatTurnReport> {
    run_chat_turn_with_events(
        input,
        persona,
        provider,
        agent,
        session,
        registry,
        ChatTurnEventOptions {
            options,
            event_sink: noop_agent_event_sink(),
            session_sink: None,
            parent_entry_id: None,
            turn_id: None,
        },
    )
    .await
}

#[derive(Clone)]
pub struct ChatTurnEventOptions<'a> {
    pub options: &'a ChatRunOptions,
    pub event_sink: &'a dyn AgentEventSink,
    pub session_sink: Option<&'a PersistingAgentTurnSink>,
    pub parent_entry_id: Option<SessionEntryId>,
    pub turn_id: Option<TurnId>,
}

pub async fn run_chat_turn_with_events(
    input: &str,
    persona: &PersonaProfile,
    provider: &dyn ModelProvider,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    event_options: ChatTurnEventOptions<'_>,
) -> Result<ChatTurnReport> {
    let options = event_options.options;
    let event_sink = event_options.event_sink;
    let chat_session_id = options
        .session_id
        .clone()
        .unwrap_or_else(new_chat_session_id);
    let session_id = SessionId::from(chat_session_id.clone());
    let turn_id = event_options.turn_id.clone().unwrap_or_default();
    let mut single_call_events = Vec::new();
    let user_entry_id = append_chat_user_session_entry(
        event_options.session_sink,
        &session_id,
        &turn_id,
        event_options.parent_entry_id,
        &agent.name,
        input,
    )?;
    if !options.agent_loop {
        emit_chat_event(
            &mut single_call_events,
            event_sink,
            &session_id,
            &turn_id,
            AgentEventSource::Runtime,
            AgentEventKind::SessionStart,
            json!({
                "agent": &agent.name,
            }),
        )?;
        emit_chat_event(
            &mut single_call_events,
            event_sink,
            &session_id,
            &turn_id,
            AgentEventSource::Runtime,
            AgentEventKind::TurnStart,
            json!({
                "agent": &agent.name,
                "stream": options.stream,
                "agent_loop": false,
            }),
        )?;
        emit_chat_event(
            &mut single_call_events,
            event_sink,
            &session_id,
            &turn_id,
            AgentEventSource::User,
            AgentEventKind::UserMessage,
            json!({
                "content": redact_secrets(input),
            }),
        )?;
    }
    let context_engine = LocalChatContextEngine;
    if let Err(error) = context_engine
        .ingest(ContextEvent {
            kind: "user_input".into(),
            scope: options.scope.clone(),
            content: redact_secrets(input),
        })
        .await
    {
        let _ = emit_chat_failure_events(
            &mut single_call_events,
            event_sink,
            &session_id,
            &turn_id,
            "context_ingest",
            &error,
        );
        return Err(error);
    }
    let chat_context = match build_chat_context_with_engine(
        &context_engine,
        input,
        agent,
        session,
        registry,
        options,
    )
    .await
    {
        Ok(context) => context,
        Err(error) => {
            let _ = emit_chat_failure_events(
                &mut single_call_events,
                event_sink,
                &session_id,
                &turn_id,
                "context_assemble",
                &error,
            );
            return Err(error);
        }
    };
    let context_chars = chat_context_char_count(&chat_context);
    let runtime_context = ContextBuilder::new()
        .persona_context(render_persona_agent_context(persona, agent))
        .relationship_context(chat_context.relationship.clone())
        .chat_history_context(chat_context.history.clone())
        .memory_context(chat_context.memory.clone())
        .rag_context(chat_context.rag.clone())
        .build();
    let system_prompt = render_chat_system_prompt(&runtime_context);
    if let Err(error) = session.audit.append(AuditEvent::new(
        "chat_context_built",
        None,
        "chat context built from persona, memory, and RAG",
        json!({
            "memory_hits": chat_context.memory.len(),
            "relationship_hits": chat_context.relationship.len(),
            "history_hits": chat_context.history.len(),
            "rag_hits": chat_context.rag.len(),
            "context_chars": context_chars,
            "context_char_budget": options.context_char_budget,
            "history_context_limit": options.history_context_limit,
            "history_summary_limit": options.history_summary_limit,
            "provider": provider.name(),
            "agent": &agent.name,
            "agent_mode": agent.mode().as_str(),
        }),
    )?) {
        let _ = emit_chat_failure_events(
            &mut single_call_events,
            event_sink,
            &session_id,
            &turn_id,
            "audit_chat_context_built",
            &error,
        );
        return Err(error);
    }
    let context_signal = if chat_context.relationship.is_empty()
        && chat_context.history.is_empty()
        && chat_context.memory.is_empty()
        && chat_context.rag.is_empty()
    {
        RuntimeSignal::Planning
    } else {
        RuntimeSignal::Research
    };
    if let Err(error) = record_emotion_signal(
        &session.audit,
        context_signal,
        "chat context prepared",
        json!({
            "memory_hits": chat_context.memory.len(),
            "relationship_hits": chat_context.relationship.len(),
            "history_hits": chat_context.history.len(),
            "rag_hits": chat_context.rag.len(),
            "agent": &agent.name,
        }),
    ) {
        let _ = emit_chat_failure_events(
            &mut single_call_events,
            event_sink,
            &session_id,
            &turn_id,
            "emotion_context_signal",
            &error,
        );
        return Err(error);
    }
    let (response, streamed, stream_chunks) = if options.agent_loop {
        let loop_report = run_agent_loop_with_events(
            AgentLoopInput {
                session_id: Some(chat_session_id.clone()),
                turn_id: Some(turn_id.to_string()),
                task_id: None,
                system_prompt,
                user_input: input.into(),
            },
            provider,
            session,
            registry,
            event_sink,
            AgentLoopOptions {
                max_iterations: 4,
                max_tokens: Some(512),
                temperature: Some(0.4),
                stream: options.stream,
                guardrails: GuardrailConfig::default(),
            },
        )
        .await?;
        let response = ModelResponse {
            provider: loop_report.provider,
            model: loop_report.model,
            content: loop_report.final_content,
            tool_calls: Vec::new(),
            usage: loop_report.usage,
        };
        (response, loop_report.streamed, loop_report.stream_chunks)
    } else if options.stream {
        let request = ModelRequest {
            messages: vec![
                ModelMessage::system(system_prompt),
                ModelMessage::user(redact_secrets(input)),
            ],
            max_tokens: Some(512),
            temperature: Some(0.4),
            tools: Vec::new(),
        };
        let stream = match provider.stream(request).await {
            Ok(stream) => stream,
            Err(error) => {
                let error = redacted_chat_error(error);
                emit_chat_failure_events(
                    &mut single_call_events,
                    event_sink,
                    &session_id,
                    &turn_id,
                    "provider_stream",
                    &error,
                )?;
                return Err(error);
            }
        };
        let response = ModelResponse {
            provider: stream.provider.clone(),
            model: stream.model.clone(),
            content: stream.content(),
            tool_calls: Vec::new(),
            usage: stream.usage.clone(),
        };
        for event in model_response_stream_events(&response) {
            emit_chat_event(
                &mut single_call_events,
                event_sink,
                &session_id,
                &turn_id,
                AgentEventSource::Model,
                AgentEventKind::ModelStream(event),
                serde_json::Value::Null,
            )?;
        }
        (response, true, stream.chunks)
    } else {
        let request = ModelRequest {
            messages: vec![
                ModelMessage::system(system_prompt),
                ModelMessage::user(redact_secrets(input)),
            ],
            max_tokens: Some(512),
            temperature: Some(0.4),
            tools: Vec::new(),
        };
        let response = match provider.generate(request).await {
            Ok(response) => response,
            Err(error) => {
                let error = redacted_chat_error(error);
                emit_chat_failure_events(
                    &mut single_call_events,
                    event_sink,
                    &session_id,
                    &turn_id,
                    "provider_generate",
                    &error,
                )?;
                return Err(error);
            }
        };
        for event in model_response_stream_events(&response) {
            emit_chat_event(
                &mut single_call_events,
                event_sink,
                &session_id,
                &turn_id,
                AgentEventSource::Model,
                AgentEventKind::ModelStream(event),
                serde_json::Value::Null,
            )?;
        }
        (response, false, Vec::new())
    };
    if let Err(error) = session.audit.append(AuditEvent::new(
        "chat_model_result",
        None,
        "chat model response generated",
        json!({
            "provider": response.provider,
            "model": response.model,
            "streamed": streamed,
            "agent_loop": options.agent_loop,
            "chunk_count": stream_chunks.len(),
            "usage": response.usage,
        }),
    )?) {
        let _ = emit_chat_failure_events(
            &mut single_call_events,
            event_sink,
            &session_id,
            &turn_id,
            "audit_chat_model_result",
            &error,
        );
        return Err(error);
    }
    let final_emotion = match record_emotion_signal(
        &session.audit,
        RuntimeSignal::TaskComplete,
        "chat response generated",
        json!({
            "provider": &response.provider,
            "model": &response.model,
            "streamed": streamed,
        }),
    ) {
        Ok(emotion) => emotion,
        Err(error) => {
            let _ = emit_chat_failure_events(
                &mut single_call_events,
                event_sink,
                &session_id,
                &turn_id,
                "emotion_signal",
                &error,
            );
            return Err(error);
        }
    };
    let relationship_learned =
        match learn_relationships_from_chat(input, session, registry, options).await {
            Ok(count) => count,
            Err(error) => {
                let _ = emit_chat_failure_events(
                    &mut single_call_events,
                    event_sink,
                    &session_id,
                    &turn_id,
                    "relationship_learning",
                    &error,
                );
                return Err(error);
            }
        };
    if let Err(error) = context_engine
        .after_turn(TurnRecord {
            session_id: Some(chat_session_id.clone()),
            user_input: redact_secrets(input),
            assistant_output: response.content.clone(),
        })
        .await
    {
        let _ = emit_chat_failure_events(
            &mut single_call_events,
            event_sink,
            &session_id,
            &turn_id,
            "context_after_turn",
            &error,
        );
        return Err(error);
    }
    if let Err(error) = append_chat_assistant_session_entry(ChatAssistantEntryInput {
        session_sink: event_options.session_sink,
        session_id: &session_id,
        turn_id: &turn_id,
        user_entry_id,
        agent: &agent.name,
        response: &response,
        streamed,
        stats: ChatSessionEntryStats {
            relationship_hits: chat_context.relationship.len(),
            memory_hits: chat_context.memory.len(),
            rag_hits: chat_context.rag.len(),
        },
    }) {
        let _ = emit_chat_failure_events(
            &mut single_call_events,
            event_sink,
            &session_id,
            &turn_id,
            "session_entry_append",
            &error,
        );
        return Err(error);
    }
    if !options.agent_loop {
        emit_chat_event(
            &mut single_call_events,
            event_sink,
            &session_id,
            &turn_id,
            AgentEventSource::Runtime,
            AgentEventKind::TurnEnd,
            json!({
                "provider": &response.provider,
                "model": &response.model,
                "streamed": streamed,
                "relationship_hits": chat_context.relationship.len(),
                "memory_hits": chat_context.memory.len(),
                "rag_hits": chat_context.rag.len(),
            }),
        )?;
    }
    let (chat_history_path, chat_session_id) = if let Some(path) = &options.chat_history_path {
        let backend = options.chat_history_backend.as_deref().unwrap_or("jsonl");
        let history_store = ChatHistoryStore::from_path_with_backend(path, backend)?;
        let record = build_chat_history_record_with_turn_id(
            turn_id.to_string(),
            ChatHistoryAppend {
                session_id: &chat_session_id,
                agent: &agent.name,
                provider: &response.provider,
                model: &response.model,
                streamed,
                user_message: input,
                assistant_message: &response.content,
                relationship_hits: chat_context.relationship.len(),
                memory_hits: chat_context.memory.len(),
                rag_hits: chat_context.rag.len(),
            },
        )?;
        if let Err(error) = history_store.append(&record) {
            let _ = emit_chat_failure_events(
                &mut single_call_events,
                event_sink,
                &session_id,
                &turn_id,
                "chat_history_append",
                &error,
            );
            return Err(error);
        }
        if let Err(error) = session.audit.append(AuditEvent::new(
            "chat_history_recorded",
            None,
            "chat turn recorded to local history",
            json!({
                "session_id": record.session_id,
                "turn_id": record.turn_id,
                "path": path,
                "agent": &agent.name,
                "provider": &response.provider,
                "model": &response.model,
                "streamed": streamed,
            }),
        )?) {
            let _ = emit_chat_failure_events(
                &mut single_call_events,
                event_sink,
                &session_id,
                &turn_id,
                "audit_chat_history_recorded",
                &error,
            );
            return Err(error);
        }
        (Some(path.clone()), Some(chat_session_id))
    } else {
        (None, None)
    };
    Ok(ChatTurnReport {
        response,
        emotion: final_emotion,
        streamed,
        stream_chunks,
        relationship_hits: chat_context.relationship.len(),
        relationship_learned,
        history_hits: chat_context.history.len(),
        memory_hits: chat_context.memory.len(),
        rag_hits: chat_context.rag.len(),
        chat_history_path,
        chat_session_id,
    })
}

struct ChatSessionEntryStats {
    relationship_hits: usize,
    memory_hits: usize,
    rag_hits: usize,
}

struct ChatAssistantEntryInput<'a> {
    session_sink: Option<&'a PersistingAgentTurnSink>,
    session_id: &'a SessionId,
    turn_id: &'a TurnId,
    user_entry_id: Option<SessionEntryId>,
    agent: &'a str,
    response: &'a ModelResponse,
    streamed: bool,
    stats: ChatSessionEntryStats,
}

fn append_chat_user_session_entry(
    session_sink: Option<&PersistingAgentTurnSink>,
    session_id: &SessionId,
    turn_id: &TurnId,
    parent_entry_id: Option<SessionEntryId>,
    agent: &str,
    user_input: &str,
) -> Result<Option<SessionEntryId>> {
    let Some(session_sink) = session_sink else {
        return Ok(None);
    };
    let redacted_user = redact_secrets(user_input);
    let mut user_entry = SessionEntry::new(session_id.clone(), SessionEntryKind::UserMessage);
    user_entry.parent_entry_id = parent_entry_id;
    user_entry.turn_id = Some(turn_id.clone());
    user_entry.visible_text = Some(redacted_user.clone());
    user_entry.payload = json!({
        "role": "user",
        "agent": redact_secrets(agent),
        "content": redacted_user,
    });
    let entry_id = user_entry.entry_id.clone();
    session_sink.append_entry(&user_entry)?;
    Ok(Some(entry_id))
}

fn append_chat_assistant_session_entry(input: ChatAssistantEntryInput<'_>) -> Result<()> {
    let Some(session_sink) = input.session_sink else {
        return Ok(());
    };
    let redacted_assistant = redact_secrets(&input.response.content);
    let mut assistant_entry =
        SessionEntry::new(input.session_id.clone(), SessionEntryKind::AssistantMessage);
    assistant_entry.parent_entry_id = input.user_entry_id;
    assistant_entry.turn_id = Some(input.turn_id.clone());
    assistant_entry.visible_text = Some(redacted_assistant.clone());
    assistant_entry.payload = json!({
        "role": "assistant",
        "agent": redact_secrets(input.agent),
        "provider": redact_secrets(&input.response.provider),
        "model": redact_secrets(&input.response.model),
        "streamed": input.streamed,
        "content": redacted_assistant,
        "relationship_hits": input.stats.relationship_hits,
        "memory_hits": input.stats.memory_hits,
        "rag_hits": input.stats.rag_hits,
        "usage": &input.response.usage,
    });
    session_sink.append_entry(&assistant_entry)
}

fn emit_chat_event(
    events: &mut Vec<AgentEvent>,
    sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    source: AgentEventSource,
    kind: AgentEventKind,
    payload: serde_json::Value,
) -> Result<()> {
    let parent_event_id = events.last().map(|event| event.event_id.clone());
    let event = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        parent_event_id,
        source,
        kind,
        payload,
    );
    sink.emit(&event)?;
    events.push(event);
    Ok(())
}

fn emit_chat_lifecycle_event(
    sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
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

fn redacted_chat_error(error: IkarosError) -> IkarosError {
    IkarosError::Message(redact_secrets(&error.to_string()))
}

fn emit_chat_failure_event(
    sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    phase: &str,
    error: &dyn std::fmt::Display,
) -> Result<()> {
    let mut events = Vec::new();
    emit_chat_failure_events(&mut events, sink, session_id, turn_id, phase, error)
}

fn emit_chat_failure_events(
    events: &mut Vec<AgentEvent>,
    sink: &dyn AgentEventSink,
    session_id: &SessionId,
    turn_id: &TurnId,
    phase: &str,
    error: &dyn std::fmt::Display,
) -> Result<()> {
    emit_chat_event(
        events,
        sink,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::Error,
        json!({
            "phase": phase,
            "message": redact_secrets(&error.to_string()),
        }),
    )?;
    emit_chat_event(
        events,
        sink,
        session_id,
        turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "status": "failed",
            "phase": phase,
        }),
    )
}

fn model_response_stream_events(response: &ModelResponse) -> Vec<ModelStreamEvent> {
    let mut events = vec![ModelStreamEvent::Start {
        provider: redact_secrets(&response.provider),
        model: redact_secrets(&response.model),
    }];
    if !response.content.is_empty() {
        events.push(ModelStreamEvent::TextDelta(redact_secrets(
            &response.content,
        )));
    }
    if response.usage.total_or_prompt_completion() > 0 {
        events.push(ModelStreamEvent::Usage(response.usage.clone()));
    }
    events.push(ModelStreamEvent::Done);
    events
}
