// SPDX-License-Identifier: GPL-3.0-only

use super::{
    context::chat_context_char_count,
    context_engine::{
        ContextEngine, ContextEvent, LocalChatContextEngine, TurnRecord,
        build_chat_context_with_engine,
    },
    history::{
        ChatHistoryAppend, ChatHistoryStore, build_chat_history_record, new_chat_session_id,
    },
    learning::learn_relationships_from_chat,
    prompt::{render_chat_system_prompt, render_persona_agent_context},
    types::{ChatMessageResult, ChatRunOptions, ChatTurnReport},
};
use crate::{AgentLoopInput, AgentLoopOptions, run_agent_loop};
use crate::{record_emotion_signal, resolve_agent_instance, session_and_registry_for_instance};
use ikaros_core::{
    ContextBuilder, IkarosConfig, IkarosPaths, ResolvedAgentProfile, Result, redact_secrets,
};
use ikaros_harness::GuardrailConfig;
use ikaros_harness::{AuditEvent, ExecutionSession, SkillRegistry};
use ikaros_memory::{LocalMemoryStore, MemoryProvider, MemoryTurnRecord, MemoryTurnStart};
use ikaros_models::{
    ModelMessage, ModelProvider, ModelRequest, ModelResponse, ModelUsageLedger,
    governed_provider_from_config,
};
use ikaros_soul::{PersonaProfile, RuntimeSignal, load_or_default};
use serde_json::json;
use std::path::Path;

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
    let provider = governed_provider_from_config(&config.model.default, &paths.audit_dir)?;
    let (session, registry) = session_and_registry_for_instance(paths, &config, &agent_instance)?;
    let memory_provider = LocalMemoryStore::new(&paths.memory_dir, &config.memory.backend)?;
    memory_provider.turn_start(MemoryTurnStart {
        session_id: options.session_id.clone(),
        agent_id: Some(agent_instance.agent_id.clone()),
        user_input: redact_secrets(message),
    })?;
    let usage_ledger = ModelUsageLedger::new(&paths.audit_dir);
    let history_store =
        ChatHistoryStore::new_with_backend(&paths.home, &config.chat_history.backend)?;
    let mut options = options;
    if options.session_id.is_none() {
        options.session_id = Some(new_chat_session_id());
    }
    options.chat_history_path = Some(history_store.path().to_path_buf());
    options.chat_history_backend = Some(history_store.backend_name().into());
    let report = run_chat_turn(
        message,
        &persona,
        provider.as_ref(),
        &agent,
        &session,
        &registry,
        &options,
    )
    .await?;
    memory_provider.sync_turn(MemoryTurnRecord {
        session_id: report.chat_session_id.clone(),
        agent_id: Some(agent_instance.agent_id.clone()),
        user_input: redact_secrets(message),
        assistant_output: report.response.content.clone(),
    })?;
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
    let context_engine = LocalChatContextEngine;
    context_engine
        .ingest(ContextEvent {
            kind: "user_input".into(),
            scope: options.scope.clone(),
            content: redact_secrets(input),
        })
        .await?;
    let chat_context =
        build_chat_context_with_engine(&context_engine, input, agent, session, registry, options)
            .await?;
    let context_chars = chat_context_char_count(&chat_context);
    let runtime_context = ContextBuilder::new()
        .persona_context(render_persona_agent_context(persona, agent))
        .relationship_context(chat_context.relationship.clone())
        .chat_history_context(chat_context.history.clone())
        .memory_context(chat_context.memory.clone())
        .rag_context(chat_context.rag.clone())
        .build();
    let system_prompt = render_chat_system_prompt(&runtime_context);
    session.audit.append(AuditEvent::new(
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
    )?)?;
    let context_signal = if chat_context.relationship.is_empty()
        && chat_context.history.is_empty()
        && chat_context.memory.is_empty()
        && chat_context.rag.is_empty()
    {
        RuntimeSignal::Planning
    } else {
        RuntimeSignal::Research
    };
    record_emotion_signal(
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
    )?;
    let (response, streamed, stream_chunks) = if options.agent_loop {
        let loop_report = run_agent_loop(
            AgentLoopInput {
                task_id: None,
                system_prompt,
                user_input: input.into(),
            },
            provider,
            session,
            registry,
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
        let stream = provider.stream(request).await?;
        let response = ModelResponse {
            provider: stream.provider.clone(),
            model: stream.model.clone(),
            content: stream.content(),
            tool_calls: Vec::new(),
            usage: stream.usage.clone(),
        };
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
        (provider.generate(request).await?, false, Vec::new())
    };
    session.audit.append(AuditEvent::new(
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
    )?)?;
    let final_emotion = record_emotion_signal(
        &session.audit,
        RuntimeSignal::TaskComplete,
        "chat response generated",
        json!({
            "provider": &response.provider,
            "model": &response.model,
            "streamed": streamed,
        }),
    )?;
    let relationship_learned =
        learn_relationships_from_chat(input, session, registry, options).await?;
    context_engine
        .after_turn(TurnRecord {
            session_id: options.session_id.clone(),
            user_input: redact_secrets(input),
            assistant_output: response.content.clone(),
        })
        .await?;
    let (chat_history_path, chat_session_id) = if let Some(path) = &options.chat_history_path {
        let backend = options.chat_history_backend.as_deref().unwrap_or("jsonl");
        let history_store = ChatHistoryStore::from_path_with_backend(path, backend)?;
        let session_id = options
            .session_id
            .clone()
            .unwrap_or_else(new_chat_session_id);
        let record = build_chat_history_record(ChatHistoryAppend {
            session_id: &session_id,
            agent: &agent.name,
            provider: &response.provider,
            model: &response.model,
            streamed,
            user_message: input,
            assistant_message: &response.content,
            relationship_hits: chat_context.relationship.len(),
            memory_hits: chat_context.memory.len(),
            rag_hits: chat_context.rag.len(),
        })?;
        history_store.append(&record)?;
        session.audit.append(AuditEvent::new(
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
        )?)?;
        (Some(path.clone()), Some(session_id))
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
