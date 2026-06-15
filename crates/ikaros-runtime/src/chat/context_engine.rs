// SPDX-License-Identifier: GPL-3.0-only

use super::{
    context::{
        apply_chat_context_token_budget, chat_context_token_count_with_default,
        context_lookup_is_safe_read, extract_memory_context, extract_rag_context,
    },
    history::ChatHistoryStore,
    types::{ChatContext, ChatRunOptions},
};
use crate::{relationship_context_lines, relationship_snapshot_from_session};
use ikaros_context::{
    ContextBudget, ContextDiff, ContextReference, HeuristicTokenEstimator, TokenEstimator,
    diff_chat_context, parse_context_references, resolve_context_references,
};
use ikaros_core::{IkarosError, ResolvedAgentProfile, Result};
use ikaros_harness::{ExecutionSession, SkillRegistry};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{future::Future, path::Path, pin::Pin};

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
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactInput {
    pub context: ChatContext,
    pub budget: ContextBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CompactReport {
    pub before_tokens: usize,
    pub after_tokens: usize,
    pub context: ChatContext,
    pub diff: ContextDiff,
    pub budget: ContextBudget,
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
            let estimator = HeuristicTokenEstimator;
            let before = input.context;
            let before_tokens = chat_context_token_count_with_default(&before);
            let context = apply_chat_context_token_budget(before.clone(), input.budget.max_tokens);
            let after_tokens = chat_context_token_count_with_default(&context);
            let mut budget = input.budget;
            budget.used_tokens = after_tokens;
            let diff = diff_chat_context(&before, &context, &estimator);
            Ok(CompactReport {
                before_tokens,
                after_tokens,
                context,
                diff,
                budget,
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
            if options.no_context {
                let context = ChatContext::default();
                return Ok(ContextBundle::from_context(
                    context.clone(),
                    context,
                    ContextBudget::unbounded(HeuristicTokenEstimator.name()),
                    Vec::new(),
                    &HeuristicTokenEstimator,
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
            let references = assemble_reference_context(
                &mut context,
                input.input,
                &input.session.sandbox.workspace_root,
            )?;

            let compacted = self
                .compact(CompactInput {
                    context: context.clone(),
                    budget: ContextBudget::new(
                        options.context_token_budget,
                        HeuristicTokenEstimator.name(),
                    ),
                })
                .await?;
            Ok(ContextBundle::from_context(
                context,
                compacted.context,
                compacted.budget,
                references,
                &HeuristicTokenEstimator,
            ))
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
        })
        .await?;
    Ok(bundle)
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
        context.memory = extract_memory_context(&result.output, options.memory_limit);
    }
    Ok(())
}

fn assemble_reference_context(
    context: &mut ChatContext,
    input: &str,
    workspace_root: &Path,
) -> Result<Vec<ContextReference>> {
    let references = parse_context_references(input);
    context.references = resolve_context_references(&references, workspace_root)
        .map_err(|error| IkarosError::Message(error.to_string()))?;
    Ok(references)
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
