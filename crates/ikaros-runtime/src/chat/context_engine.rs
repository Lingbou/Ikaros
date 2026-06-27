// SPDX-License-Identifier: GPL-3.0-only

use super::types::{ChatContext, ChatRunOptions};
use ikaros_context::TrajectoryCompressor;
use ikaros_core::{IkarosError, ResolvedAgentProfile, Result};
use ikaros_harness::{ExecutionSession, SkillRegistry};
use std::{future::Future, pin::Pin};

mod budget;
mod compact;
mod history;
mod local;
mod memory;
mod provider;
mod rag;
mod references;
mod types;

pub use budget::context_estimator_for_model;
use compact::compact_report_from_context_report;
pub use compact::{CompactInput, CompactReport};
pub use ikaros_context::ContextBundle;
pub use local::LocalChatContextEngine;
pub use provider::ProviderSummaryContextEngine;
pub use types::{ContextAssembleInput, ContextEvent, ContextModelBudget, TurnRecord};

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
            Ok(compact_report_from_context_report(compressed))
        })
    }

    fn after_turn<'a>(
        &'a self,
        _turn: TurnRecord,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        Box::pin(async { Ok(()) })
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
