// SPDX-License-Identifier: GPL-3.0-only

use super::{
    context::{
        context_lookup_is_safe_read, extract_projection_context, extract_rag_context,
        extract_retrieved_memory_context, extract_working_memory_context, redact_chat_context,
    },
    history::ChatHistoryStore,
    types::{ChatContext, ChatRunOptions},
};
use crate::{relationship_context_lines, relationship_snapshot_from_session};
use ikaros_context::{
    ContextBudget, ContextCompressedSection, ContextDiff, ContextReference, ContextReferenceKind,
    ContextTokenEstimator, ContextTokenizerKind, TokenEstimator, TrajectoryCompressor,
    parse_context_references, resolve_context_reference,
};
use ikaros_core::{IkarosError, ResolvedAgentProfile, Result, redact_secrets};
use ikaros_harness::{ExecutionSession, NetworkEgressRequest, SkillRegistry};
use ikaros_models::{ModelContextProfile, ModelTokenizerKind};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{future::Future, pin::Pin};

pub use ikaros_context::ContextBundle;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContextEvent {
    pub kind: String,
    pub scope: Option<String>,
    pub content: String,
}

pub struct ContextAssembleInput<'a> {
    pub input: &'a str,
    pub agent: &'a ResolvedAgentProfile,
    pub session: &'a ExecutionSession,
    pub registry: &'a SkillRegistry,
    pub options: &'a ChatRunOptions,
    pub model_context: Option<&'a ModelContextProfile>,
    pub reserved_system_tokens: u32,
}

pub struct ContextModelBudget<'a> {
    pub model_context: &'a ModelContextProfile,
    pub reserved_system_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactInput {
    pub context: ChatContext,
    pub budget: ContextBudget,
    pub tokenizer: ContextTokenizerKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactReport {
    pub before_tokens: usize,
    pub after_tokens: usize,
    pub context: ChatContext,
    pub diff: ContextDiff,
    pub budget: ContextBudget,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compressed_sections: Vec<ContextCompressedSection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnRecord {
    pub session_id: Option<String>,
    pub user_input: String,
    pub assistant_output: String,
}

pub trait ContextEngine: Send + Sync {
    fn ingest<'a>(
        &'a self,
        _event: ContextEvent,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn assemble<'a>(
        &'a self,
        input: ContextAssembleInput<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ContextBundle>> + 'a>>;

    fn compact<'a>(
        &'a self,
        input: CompactInput,
    ) -> Pin<Box<dyn Future<Output = Result<CompactReport>> + 'a>> {
        Box::pin(async move {
            let estimator = input.tokenizer.estimator();
            let before = input.context;
            let compressed = TrajectoryCompressor::default()
                .compress(before.clone(), input.budget, &estimator)
                .map_err(|error| IkarosError::Message(error.to_string()))?;
            Ok(CompactReport {
                before_tokens: compressed.before_tokens,
                after_tokens: compressed.after_tokens,
                context: compressed.context,
                diff: compressed.diff,
                budget: compressed.budget,
                compressed_sections: compressed.compressed_sections,
                summary: compressed.summary,
                continuation_prompt: compressed.continuation_prompt,
            })
        })
    }

    fn after_turn<'a>(
        &'a self,
        _turn: TurnRecord,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LocalChatContextEngine;

impl ContextEngine for LocalChatContextEngine {
    fn assemble<'a>(
        &'a self,
        input: ContextAssembleInput<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ContextBundle>> + 'a>> {
        Box::pin(async move {
            let options = input.options;
            let estimator = context_estimator_for_input(&input);
            if options.no_context {
                let context = ChatContext::default();
                return Ok(ContextBundle::from_context(
                    context.clone(),
                    context,
                    ContextBudget::unbounded(estimator.name()),
                    Vec::new(),
                    &estimator,
                ));
            }

            let mut context = ChatContext::default();
            assemble_history_context(&mut context, options)?;
            assemble_memory_context(
                &mut context,
                input.input,
                input.agent,
                input.session,
                input.registry,
                options,
            )
            .await?;
            assemble_rag_context(
                &mut context,
                input.input,
                input.agent,
                input.session,
                input.registry,
                options,
            )
            .await?;
            let references =
                assemble_reference_context(&mut context, input.input, input.session).await?;

            let context = redact_chat_context(context);
            let compacted = self
                .compact(CompactInput {
                    context: context.clone(),
                    budget: context_budget_for_input(&input, estimator.name()),
                    tokenizer: estimator.kind(),
                })
                .await?;
            let mut bundle = ContextBundle::from_context(
                context,
                compacted.context,
                compacted.budget,
                references,
                &estimator,
            );
            bundle.diff = compacted.diff;
            bundle.compressed_sections = compacted.compressed_sections;
            bundle.compression_summary = compacted.summary;
            bundle.continuation_prompt = compacted.continuation_prompt;
            Ok(bundle)
        })
    }
}

pub async fn build_chat_context(
    input: &str,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<ChatContext> {
    build_chat_context_with_engine(
        &LocalChatContextEngine,
        input,
        agent,
        session,
        registry,
        options,
    )
    .await
}

pub async fn build_chat_context_with_engine(
    engine: &dyn ContextEngine,
    input: &str,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<ChatContext> {
    let bundle =
        build_chat_context_bundle_with_engine(engine, input, agent, session, registry, options)
            .await?;
    Ok(bundle.context)
}

pub async fn build_chat_context_bundle_with_engine(
    engine: &dyn ContextEngine,
    input: &str,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<ContextBundle> {
    let bundle = engine
        .assemble(ContextAssembleInput {
            input,
            agent,
            session,
            registry,
            options,
            model_context: None,
            reserved_system_tokens: 0,
        })
        .await?;
    Ok(bundle)
}

pub async fn build_chat_context_bundle_with_model_context(
    engine: &dyn ContextEngine,
    input: &str,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
    model_budget: ContextModelBudget<'_>,
) -> Result<ContextBundle> {
    let bundle = engine
        .assemble(ContextAssembleInput {
            input,
            agent,
            session,
            registry,
            options,
            model_context: Some(model_budget.model_context),
            reserved_system_tokens: model_budget.reserved_system_tokens,
        })
        .await?;
    Ok(bundle)
}

fn context_budget_for_input(
    input: &ContextAssembleInput<'_>,
    estimator: impl Into<String>,
) -> ContextBudget {
    let requested = input.options.context_token_budget;
    let Some(model_context) = input.model_context else {
        return ContextBudget::new(requested, estimator);
    };
    let available = model_context
        .available_context_tokens(input.reserved_system_tokens)
        .max(1) as usize;
    let max_tokens = if requested == 0 {
        available
    } else {
        requested.min(available)
    };
    ContextBudget::new(max_tokens, estimator).with_model_window(
        requested,
        model_context.context_window,
        model_context.default_output_tokens,
        input.reserved_system_tokens,
        model_context.source.clone(),
    )
}

pub fn context_estimator_for_model(
    model_context: Option<&ModelContextProfile>,
) -> ContextTokenEstimator {
    context_tokenizer_for_model(model_context).estimator()
}

pub fn context_tokenizer_for_model(
    model_context: Option<&ModelContextProfile>,
) -> ContextTokenizerKind {
    match model_context.map(|context| context.tokenizer) {
        Some(ModelTokenizerKind::OpenAiCompatible) => ContextTokenizerKind::OpenAiCompatible,
        Some(ModelTokenizerKind::Anthropic) => ContextTokenizerKind::AnthropicFallback,
        Some(ModelTokenizerKind::Ollama) => ContextTokenizerKind::OllamaFallback,
        Some(ModelTokenizerKind::Mock) => ContextTokenizerKind::Mock,
        Some(ModelTokenizerKind::Heuristic) | None => ContextTokenizerKind::Heuristic,
    }
}

fn context_estimator_for_input(input: &ContextAssembleInput<'_>) -> ContextTokenEstimator {
    context_estimator_for_model(input.model_context)
}

fn assemble_history_context(context: &mut ChatContext, options: &ChatRunOptions) -> Result<()> {
    if options.history_context_limit == 0 {
        return Ok(());
    }
    let Some(path) = &options.chat_history_path else {
        return Ok(());
    };
    let backend = options.chat_history_backend.as_deref().unwrap_or("jsonl");
    let store = ChatHistoryStore::from_path_with_backend(path, backend)?;
    context.history = if let Some(session_id) = options.session_id.as_deref() {
        store.context_lines_for_session(
            session_id,
            options.history_context_limit,
            options.history_summary_limit,
        )?
    } else {
        store.context_lines(options.history_context_limit, options.history_summary_limit)?
    };
    Ok(())
}

async fn assemble_memory_context(
    context: &mut ChatContext,
    input: &str,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<()> {
    if !agent.profile.memory_context || options.memory_limit == 0 {
        return Ok(());
    }
    let relationship = relationship_snapshot_from_session(
        session,
        registry,
        options.scope.as_deref(),
        options.memory_limit,
    )
    .await?;
    context.relationship = relationship_context_lines(&relationship, options.memory_limit);

    if context_lookup_is_safe_read(registry, "memory_projection") {
        let mut projection_input = json!({
            "user_scope": "default",
        });
        let mut projection_audit_input = json!({
            "user_scope": "default",
        });
        if let Some(scope) = &options.scope {
            projection_input["project_scope"] = json!(scope);
            projection_audit_input["project_scope"] = json!(scope);
        }
        let result = session
            .execute_read_skill_with_audit_input(
                registry,
                "memory_projection",
                projection_input,
                projection_audit_input,
            )
            .await?;
        if result.ok {
            context
                .memory_projection
                .extend(extract_projection_context(&result.output));
        }
    }

    if let Some(session_id) = &options.session_id
        && context_lookup_is_safe_read(registry, "working_memory_list")
    {
        let result = session
            .execute_read_skill_with_audit_input(
                registry,
                "working_memory_list",
                json!({
                    "session_id": session_id,
                    "limit": options.memory_limit,
                }),
                json!({
                    "session_id": "<redacted chat session>",
                    "limit": options.memory_limit,
                }),
            )
            .await?;
        if result.ok {
            context
                .working_memory
                .extend(extract_working_memory_context(
                    &result.output,
                    options.memory_limit,
                ));
        }
    }

    let mut memory_input = json!({
        "query": input,
        "limit": options.memory_limit,
    });
    let mut memory_audit_input = json!({
        "query": "<redacted chat query>",
        "limit": options.memory_limit,
    });
    if let Some(scope) = &options.scope {
        memory_input["scope"] = json!(scope);
        memory_audit_input["scope"] = json!(scope);
    }
    let result = session
        .execute_read_skill_with_audit_input(
            registry,
            "memory_search",
            memory_input,
            memory_audit_input,
        )
        .await?;
    if result.ok {
        context
            .retrieved_memory
            .extend(extract_retrieved_memory_context(
                &result.output,
                options.memory_limit,
            ));
    }
    Ok(())
}

async fn assemble_reference_context(
    context: &mut ChatContext,
    input: &str,
    session: &ExecutionSession,
) -> Result<Vec<ContextReference>> {
    let references = parse_context_references(input);
    let mut resolved = Vec::with_capacity(references.len());
    for reference in &references {
        let reference_text = match &reference.kind {
            ContextReferenceKind::Url { url } => resolve_url_reference(url, session).await?,
            _ => resolve_context_reference(reference, &session.sandbox.workspace_root)
                .map_err(|error| IkarosError::Message(error.to_string()))?,
        };
        resolved.push(reference_text);
    }
    context.references = resolved;
    Ok(references)
}

async fn resolve_url_reference(url: &str, session: &ExecutionSession) -> Result<String> {
    let response = session
        .env
        .send_network_request(NetworkEgressRequest {
            method: "GET".into(),
            url: url.into(),
            headers: Default::default(),
            body: None,
        })
        .await?;
    let body = truncate_url_reference_body(&redact_secrets(&response.body));
    Ok(redact_secrets(&format!(
        "[reference/url] {url} status={}\n{}",
        response.status, body
    )))
}

fn truncate_url_reference_body(body: &str) -> String {
    const MAX_CHARS: usize = 16 * 1024;
    let mut chars = body.chars();
    let mut truncated = chars.by_ref().take(MAX_CHARS).collect::<String>();
    if chars.next().is_some() {
        truncated.push_str("\n[reference/url] truncated");
    }
    truncated
}

async fn assemble_rag_context(
    context: &mut ChatContext,
    input: &str,
    agent: &ResolvedAgentProfile,
    session: &ExecutionSession,
    registry: &SkillRegistry,
    options: &ChatRunOptions,
) -> Result<()> {
    if !agent.profile.rag_context
        || options.rag_top_k == 0
        || !context_lookup_is_safe_read(registry, "rag_search")
    {
        return Ok(());
    }
    let mut rag_input = json!({
        "query": input,
        "top_k": options.rag_top_k,
    });
    let mut rag_audit_input = json!({
        "query": "<redacted chat query>",
        "top_k": options.rag_top_k,
    });
    if let Some(scope) = &options.scope {
        rag_input["scope"] = json!(scope);
        rag_audit_input["scope"] = json!(scope);
    }
    let result = session
        .execute_read_skill_with_audit_input(registry, "rag_search", rag_input, rag_audit_input)
        .await?;
    if result.ok {
        context.rag = extract_rag_context(&result.output, options.rag_top_k);
    }
    Ok(())
}
