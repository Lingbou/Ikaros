// SPDX-License-Identifier: GPL-3.0-only

use super::{
    ChatTurnEventOptions,
    events::{emit_chat_event, emit_chat_failure_events},
    resolve_runtime_context_engine,
    session_entries::append_context_compaction_session_entry,
    setup::ChatTurnSetup,
};
use crate::{AgentEventSink, record_emotion_signal_with_correlation};
use ikaros_context::{ContextEngineKind, PromptBuildMetadata, PromptCachePlan, TokenEstimator};
use ikaros_core::{ContextBuilder, ResolvedAgentProfile, Result, redact_secrets};
use ikaros_harness::{AuditEvent, ExecutionSession, SkillRegistry};
use ikaros_models::{ModelProvider, ProviderRegistry};
use ikaros_session::{AgentEvent, AgentEventKind, AgentEventSource, SessionEntryId};
use ikaros_soul::{PersonaProfile, RuntimeSignal};
use serde_json::json;

use crate::chat::{
    context_engine::{
        ContextEngine, ContextEvent, ContextModelBudget, LocalChatContextEngine,
        ProviderSummaryContextEngine, TurnRecord, build_chat_context_bundle_with_model_context,
        context_estimator_for_model,
    },
    prompt::{build_chat_system_prompt, render_persona_agent_context},
    types::{ChatContext, ChatRunOptions},
};

pub(super) struct PreparedChatContext<'a> {
    pub(super) engine: TurnContextEngine<'a>,
    pub(super) chat_context: ChatContext,
    pub(super) system_prompt: String,
    pub(super) system_prompt_messages: Vec<String>,
    pub(super) assistant_parent_entry_id: Option<SessionEntryId>,
}

pub(super) enum TurnContextEngine<'a> {
    Local(LocalChatContextEngine),
    Summary(ProviderSummaryContextEngine<'a>),
}

impl TurnContextEngine<'_> {
    pub(super) async fn after_turn(&self, turn: TurnRecord) -> Result<()> {
        self.as_engine().after_turn(turn).await
    }

    fn as_engine(&self) -> &dyn ContextEngine {
        match self {
            Self::Local(engine) => engine,
            Self::Summary(engine) => engine,
        }
    }
}

pub(super) struct ContextPrepareInput<'provider, 'turn, 'options> {
    pub(super) input: &'turn str,
    pub(super) persona: &'turn PersonaProfile,
    pub(super) provider: &'provider dyn ModelProvider,
    pub(super) agent: &'turn ResolvedAgentProfile,
    pub(super) session: &'turn ExecutionSession,
    pub(super) registry: &'turn SkillRegistry,
    pub(super) event_options: &'turn ChatTurnEventOptions<'options>,
    pub(super) setup: &'turn ChatTurnSetup,
}

pub(super) async fn prepare_chat_context<'provider, 'turn, 'options>(
    input: ContextPrepareInput<'provider, 'turn, 'options>,
    events: &mut Vec<AgentEvent>,
) -> Result<PreparedChatContext<'provider>> {
    let options = input.event_options.options;
    let engine = context_engine_for_turn(options, input.provider)?;
    let model_context = input.provider.context_profile();
    let persona_context = render_persona_agent_context(input.persona, input.agent);
    let context_estimator = context_estimator_for_model(Some(&model_context));
    let reserved_system_tokens = context_estimator.estimate_tokens(&persona_context) as u32;
    if let Err(error) = engine
        .as_engine()
        .ingest(ContextEvent {
            kind: "user_input".into(),
            scope: options.scope.clone(),
            content: redact_secrets(input.input),
        })
        .await
    {
        let _ = emit_chat_failure_events(
            events,
            input.event_options.event_sink,
            &input.setup.session_id,
            &input.setup.turn_id,
            "context_ingest",
            &error,
        );
        return Err(error);
    }
    let context_bundle = match build_chat_context_bundle_with_model_context(
        engine.as_engine(),
        input.input,
        input.agent,
        input.session,
        input.registry,
        options,
        ContextModelBudget {
            model_context: &model_context,
            reserved_system_tokens,
        },
    )
    .await
    {
        Ok(context) => context,
        Err(error) => {
            let _ = emit_chat_failure_events(
                events,
                input.event_options.event_sink,
                &input.setup.session_id,
                &input.setup.turn_id,
                "context_assemble",
                &error,
            );
            return Err(error);
        }
    };
    let chat_context = context_bundle.context.clone();
    let context_tokens = context_bundle.budget.used_tokens;
    let runtime_context = ContextBuilder::new()
        .persona_context(persona_context)
        .relationship_context(chat_context.relationship.clone())
        .reference_context(chat_context.references.clone())
        .chat_history_context(chat_context.history.clone())
        .memory_projection_context(chat_context.memory_projection.clone())
        .working_memory_context(chat_context.working_memory.clone())
        .retrieved_memory_context(chat_context.retrieved_memory.clone())
        .rag_context(chat_context.rag.clone())
        .context_continuation_prompt(context_bundle.continuation_prompt.clone())
        .build();
    let prompt_report = build_chat_system_prompt(&runtime_context, &context_estimator);
    let prompt_metadata = prompt_report.metadata();
    let prompt_cache =
        prompt_report.prompt_cache_plan(prompt_cache_policy_for_provider(input.provider));
    let system_prompt_messages = prompt_report.system_messages_for_prompt_cache();
    let system_prompt = prompt_report.prompt.clone();
    emit_context_events(
        events,
        input.event_options.event_sink,
        &input.setup,
        &context_bundle,
        &prompt_metadata,
        &prompt_cache,
    )?;
    let assistant_parent_entry_id = append_context_compaction_session_entry(
        input.event_options.session_sink,
        &input.setup.session_id,
        &input.setup.turn_id,
        input.setup.user_entry_id.clone(),
        &context_bundle,
    )?
    .or_else(|| input.setup.user_entry_id.clone());
    audit_context_prepared(
        events,
        input.event_options.event_sink,
        input.session,
        input.agent,
        input.provider,
        options,
        &input.setup,
        &chat_context,
        context_tokens,
        &context_bundle.budget,
        &prompt_metadata,
        &prompt_cache,
    )?;
    record_context_emotion_signal(
        events,
        input.event_options.event_sink,
        input.session,
        input.agent,
        &input.setup,
        &chat_context,
    )?;
    Ok(PreparedChatContext {
        engine,
        chat_context,
        system_prompt,
        system_prompt_messages,
        assistant_parent_entry_id,
    })
}

fn context_engine_for_turn<'a>(
    options: &ChatRunOptions,
    provider: &'a dyn ModelProvider,
) -> Result<TurnContextEngine<'a>> {
    let selected_context_engine =
        resolve_runtime_context_engine(options.context_engine.as_deref())?;
    Ok(match selected_context_engine.kind {
        ContextEngineKind::DeterministicCompressor => {
            TurnContextEngine::Local(LocalChatContextEngine)
        }
        ContextEngineKind::LlmSummaryCompressor => {
            TurnContextEngine::Summary(ProviderSummaryContextEngine::new(provider))
        }
    })
}

fn prompt_cache_policy_for_provider(provider: &dyn ModelProvider) -> String {
    ProviderRegistry
        .descriptor(provider.name(), "", provider.model_id())
        .map(|descriptor| descriptor.profile_policy.prompt_cache)
        .unwrap_or_else(|_| "none".into())
}

fn emit_context_events(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    setup: &ChatTurnSetup,
    context_bundle: &ikaros_context::ContextBundle,
    prompt_metadata: &PromptBuildMetadata,
    prompt_cache: &PromptCachePlan,
) -> Result<()> {
    emit_chat_event(
        events,
        event_sink,
        &setup.session_id,
        &setup.turn_id,
        AgentEventSource::Context,
        AgentEventKind::ContextDiff,
        json!({
            "correlation_id": &setup.correlation_id,
            "budget": &context_bundle.budget,
            "diff": &context_bundle.diff,
            "compressed_sections": &context_bundle.compressed_sections,
            "compression_summary": &context_bundle.compression_summary,
            "continuation_prompt": &context_bundle.continuation_prompt,
            "references": &context_bundle.references,
            "sections": &context_bundle.sections,
            "prompt_sections": &prompt_metadata.sections,
            "prompt_estimated_tokens": prompt_metadata.estimated_tokens,
            "prompt_stable_prefix_message_count": prompt_metadata.stable_prefix_message_count,
            "prompt_stable_prefix_estimated_tokens": prompt_metadata.stable_prefix_estimated_tokens,
            "prompt_stable_prefix_hash": &prompt_metadata.stable_prefix_hash,
            "prompt_cache": prompt_cache,
        }),
    )?;
    if !context_bundle.compressed_sections.is_empty() {
        emit_chat_event(
            events,
            event_sink,
            &setup.session_id,
            &setup.turn_id,
            AgentEventSource::Context,
            AgentEventKind::ContextCompacted,
            json!({
                "summary": &context_bundle.compression_summary,
                "continuation_prompt": &context_bundle.continuation_prompt,
                "compressed_sections": &context_bundle.compressed_sections,
                "budget": &context_bundle.budget,
            }),
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn audit_context_prepared(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    session: &ExecutionSession,
    agent: &ResolvedAgentProfile,
    provider: &dyn ModelProvider,
    options: &ChatRunOptions,
    setup: &ChatTurnSetup,
    chat_context: &ChatContext,
    context_tokens: usize,
    context_budget: &ikaros_context::ContextBudget,
    prompt_metadata: &PromptBuildMetadata,
    prompt_cache: &PromptCachePlan,
) -> Result<()> {
    if let Err(error) = session.audit.append(AuditEvent::new(
        "chat_context_built",
        None,
        "chat context built from persona, memory, and RAG",
        json!({
            "correlation_id": &setup.correlation_id,
            "memory_hits": chat_context.memory_hits(),
            "relationship_hits": chat_context.relationship.len(),
            "reference_hits": chat_context.references.len(),
            "history_hits": chat_context.history.len(),
            "rag_hits": chat_context.rag.len(),
            "context_tokens": context_tokens,
            "context_budget": context_budget,
            "prompt_estimated_tokens": prompt_metadata.estimated_tokens,
            "prompt_sections": &prompt_metadata.sections,
            "prompt_stable_prefix_message_count": prompt_metadata.stable_prefix_message_count,
            "prompt_stable_prefix_estimated_tokens": prompt_metadata.stable_prefix_estimated_tokens,
            "prompt_stable_prefix_hash": &prompt_metadata.stable_prefix_hash,
            "prompt_cache": prompt_cache,
            "history_context_limit": options.history_context_limit,
            "history_summary_limit": options.history_summary_limit,
            "content_block_count": setup.content_block_count,
            "agent_loop_requested": options.agent_loop,
            "agent_loop_effective": setup.effective_agent_loop,
            "provider": provider.name(),
            "agent": &agent.name,
            "agent_mode": agent.mode().as_str(),
        }),
    )?.with_correlation_id(&setup.correlation_id)) {
        let _ = emit_chat_failure_events(
            events,
            event_sink,
            &setup.session_id,
            &setup.turn_id,
            "audit_chat_context_built",
            &error,
        );
        return Err(error);
    }
    Ok(())
}

fn record_context_emotion_signal(
    events: &mut Vec<AgentEvent>,
    event_sink: &dyn AgentEventSink,
    session: &ExecutionSession,
    agent: &ResolvedAgentProfile,
    setup: &ChatTurnSetup,
    chat_context: &ChatContext,
) -> Result<()> {
    let context_signal = if chat_context.relationship.is_empty()
        && chat_context.history.is_empty()
        && chat_context.memory_hits() == 0
        && chat_context.rag.is_empty()
    {
        RuntimeSignal::Planning
    } else {
        RuntimeSignal::Research
    };
    if let Err(error) = record_emotion_signal_with_correlation(
        &session.audit,
        context_signal,
        "chat context prepared",
        json!({
            "memory_hits": chat_context.memory_hits(),
            "relationship_hits": chat_context.relationship.len(),
            "reference_hits": chat_context.references.len(),
            "history_hits": chat_context.history.len(),
            "rag_hits": chat_context.rag.len(),
            "agent": &agent.name,
        }),
        Some(&setup.correlation_id),
    ) {
        let _ = emit_chat_failure_events(
            events,
            event_sink,
            &setup.session_id,
            &setup.turn_id,
            "emotion_context_signal",
            &error,
        );
        return Err(error);
    }
    Ok(())
}
