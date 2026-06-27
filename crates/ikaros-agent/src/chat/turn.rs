// SPDX-License-Identifier: GPL-3.0-only

use super::{
    history::new_chat_session_id,
    types::{ChatMessageResult, ChatRunOptions, ChatTurnReport},
};
use crate::agent_loop::{AgentEventSink, agent_toolset_selection, noop_agent_event_sink};
use ikaros_core::{IkarosError, PersonaProfile, ResolvedAgentProfile, Result, redact_secrets};
use ikaros_execution::harness::{ExecutionSession, SkillRegistry};
use ikaros_execution::harness::{Toolset, ToolsetSelection};
use ikaros_protocol::context::{ContextEngineDescriptor, ContextEngineRegistry};
use ikaros_providers::model::{ModelProvider, ModelRequestOptions};
use ikaros_state::memory::{
    MemoryJournal, MemoryPolicy, MemoryProvider, MemoryTurnRecord, MemoryTurnStart,
};
use ikaros_state::session::{
    AgentEventKind, AgentEventSource, PersistingAgentTurnSink, SessionEntryId, SessionId,
    SessionInputAdmission, SessionSource, SessionStore, TurnId,
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

mod agent_loop;
mod context_prepare;
mod events;
mod memory_lifecycle;
mod model;
mod result;
mod session_entries;
mod setup;
mod single_call;

use events::{emit_chat_failure_event, emit_chat_lifecycle_event};
use result::CompleteChatTurnInput;

pub use events::emit_memory_lifecycle_report as emit_chat_memory_lifecycle_report;
pub use memory_lifecycle::{
    apply_runtime_memory_policy as apply_chat_memory_policy,
    memory_policy_from_config as chat_memory_policy_from_config,
};

fn resolve_runtime_context_engine(requested: Option<&str>) -> Result<ContextEngineDescriptor> {
    let registry = ContextEngineRegistry;
    if let Some(engine) = requested {
        return registry.descriptor(engine).ok_or_else(|| {
            let supported = registry.supported_ids().join(", ");
            IkarosError::Message(format!(
                "unknown context engine `{}`; supported engines are {supported}",
                redact_secrets(engine)
            ))
        });
    }

    registry
        .default_descriptor()
        .ok_or_else(|| IkarosError::Message("context engine registry has no default engine".into()))
}

pub struct ChatMessageContext<'a> {
    pub persona: &'a PersonaProfile,
    pub provider: &'a dyn ModelProvider,
    pub agent: &'a ResolvedAgentProfile,
    pub session: &'a ExecutionSession,
    pub registry: &'a SkillRegistry,
    pub session_store: Arc<dyn SessionStore>,
    pub session_source: SessionSource,
    pub agent_id: &'a str,
    pub workspace: &'a Path,
    pub memory_provider: &'a dyn MemoryProvider,
    pub memory_journal: &'a dyn MemoryJournal,
    pub memory_policy: &'a MemoryPolicy,
    pub request_options: &'a ModelRequestOptions,
    pub model_usage_path: PathBuf,
    pub session_state_db: PathBuf,
}

pub async fn run_chat_message_with_context(
    message: &str,
    mut options: ChatRunOptions,
    context: ChatMessageContext<'_>,
) -> Result<ChatMessageResult> {
    if options.session_id.is_none() {
        options.session_id = Some(new_chat_session_id());
    }
    let chat_session_id = options
        .session_id
        .clone()
        .expect("chat session id initialized");
    let turn_id = options
        .turn_id
        .clone()
        .map(TurnId::from)
        .unwrap_or_default();
    let session_id = SessionId::from(chat_session_id.clone());
    options.session_state_db = Some(context.session_state_db.clone());
    let parent_entry_id = context
        .session_store
        .get_session(&session_id)?
        .and_then(|session| session.active_leaf_entry_id);
    let admitted_input = context
        .session_store
        .admit_input(&SessionInputAdmission::new(
            session_id.clone(),
            json!({
                "role": "user",
                "content": redact_secrets(message),
                "content_block_count": options.content_blocks.len(),
            }),
        ))?;
    let session_source = options
        .session_source
        .clone()
        .unwrap_or_else(|| context.session_source.clone());
    let event_sink = PersistingAgentTurnSink::new(context.session_store.clone())
        .with_source(session_source)
        .with_agent_id(context.agent_id.to_owned())
        .with_workspace(context.workspace.to_path_buf());
    event_sink.promote_input_on_commit(admitted_input.input_id.clone())?;
    let turn_start_report = context.memory_provider.turn_start(MemoryTurnStart {
        session_id: options.session_id.clone(),
        agent_id: Some(context.agent_id.to_owned()),
        user_input: redact_secrets(message),
    })?;
    let report = match run_chat_turn_with_events(
        message,
        context.persona,
        context.provider,
        context.agent,
        context.session,
        context.registry,
        ChatTurnEventOptions {
            options: &options,
            request_options: Some(context.request_options),
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
            let _ = context
                .session_store
                .cancel_input(&admitted_input.input_id, "chat_turn_failed");
            return Err(error);
        }
    };
    emit_chat_memory_lifecycle_report(
        &event_sink,
        &session_id,
        &turn_id,
        context.agent_id,
        &chat_session_id,
        &turn_start_report,
    )?;
    let sync_report = match context.memory_provider.sync_turn(MemoryTurnRecord {
        session_id: report.chat_session_id.clone(),
        turn_id: Some(turn_id.to_string()),
        agent_id: Some(context.agent_id.to_owned()),
        user_input: redact_secrets(message),
        assistant_output: report.response.content.clone(),
    }) {
        Ok(report) => report,
        Err(error) => {
            let _ =
                emit_chat_failure_event(&event_sink, &session_id, &turn_id, "memory_sync", &error);
            if event_sink.commit().is_err() {
                let _ = event_sink.rollback();
            }
            let _ = context
                .session_store
                .cancel_input(&admitted_input.input_id, "memory_sync_failed");
            return Err(error);
        }
    };
    emit_chat_memory_lifecycle_report(
        &event_sink,
        &session_id,
        &turn_id,
        context.agent_id,
        &chat_session_id,
        &sync_report,
    )?;
    apply_chat_memory_policy(
        context.memory_provider,
        context.memory_journal,
        context.memory_policy,
        &sync_report,
    )?;
    emit_chat_lifecycle_event(
        &event_sink,
        &session_id,
        &turn_id,
        AgentEventSource::Audit,
        AgentEventKind::AuditAnchor,
        json!({
            "audit_path": context.session.audit.path().display().to_string(),
            "model_usage_path": context.model_usage_path.display().to_string(),
            "chat_timeline": "session_store",
        }),
    )?;
    if let Err(error) = event_sink.commit() {
        let _ = context
            .session_store
            .cancel_input(&admitted_input.input_id, "chat_turn_commit_failed");
        return Err(error);
    }
    let chat_session_id = report
        .chat_session_id
        .clone()
        .or_else(|| options.session_id.clone())
        .unwrap_or_else(new_chat_session_id);
    let response = report.response;
    Ok(ChatMessageResult {
        content: response.content,
        provider: response.provider,
        model: response.model,
        emotion: report.emotion,
        streamed: report.streamed,
        stream_chunks: report.stream_chunks,
        relationship_hits: report.relationship_hits,
        relationship_candidates_created: report.relationship_candidates_created,
        reference_hits: report.reference_hits,
        history_hits: report.history_hits,
        memory_hits: report.memory_hits,
        rag_hits: report.rag_hits,
        audit_path: context.session.audit.path().to_path_buf(),
        model_usage_path: context.model_usage_path,
        session_state_db: context.session_state_db,
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
            request_options: None,
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
    pub request_options: Option<&'a ModelRequestOptions>,
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
    let mut setup = setup::setup_chat_turn(input, provider, agent, session, &event_options)?;
    let correlated_session = setup.session.clone();
    let mut single_call_events = std::mem::take(&mut setup.single_call_events);
    let prepared = context_prepare::prepare_chat_context(
        context_prepare::ContextPrepareInput {
            input,
            persona,
            provider,
            agent,
            session: &correlated_session,
            registry,
            event_options: &event_options,
            setup: &setup,
        },
        &mut single_call_events,
    )
    .await?;
    let toolsets = if options.safe_tools {
        ToolsetSelection::new([Toolset::Core])
    } else {
        agent_toolset_selection(agent)?
    };
    let model_result = if setup.effective_agent_loop {
        agent_loop::run_agent_loop(agent_loop::AgentLoopInput {
            input,
            provider,
            session: &correlated_session,
            registry,
            event_sink: event_options.event_sink,
            setup: &setup,
            request_options: setup.request_options.clone(),
            stream: options.stream,
            cancellation: options.cancellation.clone(),
            system_prompt: prepared.system_prompt.clone(),
            system_prompt_messages: prepared.system_prompt_messages.clone(),
            toolsets,
        })
        .await?
    } else {
        single_call::run_single_call(
            single_call::SingleCallInput {
                input,
                provider,
                options,
                request_options: setup.request_options.clone(),
                system_prompt_messages: &prepared.system_prompt_messages,
                event_sink: event_options.event_sink,
                setup: &setup,
            },
            &mut single_call_events,
        )
        .await?
    };
    setup.single_call_events = single_call_events;
    result::complete_chat_turn(CompleteChatTurnInput {
        input,
        agent,
        session: &correlated_session,
        registry,
        options,
        event_options: &event_options,
        setup,
        prepared,
        model_result,
    })
    .await
}
