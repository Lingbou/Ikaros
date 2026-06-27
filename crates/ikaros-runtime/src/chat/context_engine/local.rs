// SPDX-License-Identifier: GPL-3.0-only

use super::{
    CompactInput, ContextBundle, ContextEngine,
    budget::{cap_reference_context, context_budget_for_input, context_estimator_for_input},
    history::assemble_history_context,
    memory::assemble_memory_context,
    rag::assemble_rag_context,
    references::assemble_reference_context,
    types::ContextAssembleInput,
};
use crate::chat::{context::redact_chat_context, types::ChatContext};
use ikaros_context::{ContextBudget, TokenEstimator};
use ikaros_core::Result;
use std::{future::Future, pin::Pin};

#[derive(Debug, Clone, Copy, Default)]
pub struct LocalChatContextEngine;

impl ContextEngine for LocalChatContextEngine {
    fn assemble<'a>(
        &'a self,
        input: ContextAssembleInput<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ContextBundle>> + 'a>> {
        Box::pin(async move { assemble_chat_context_with_engine(self, input).await })
    }
}

pub(super) async fn assemble_chat_context_with_engine(
    engine: &dyn ContextEngine,
    input: ContextAssembleInput<'_>,
) -> Result<ContextBundle> {
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
    let references = assemble_reference_context(&mut context, input.input, input.session).await?;

    let mut context = redact_chat_context(context);
    let budget = context_budget_for_input(&input, estimator.name());
    cap_reference_context(&mut context.references, &budget, &estimator);
    let compacted = engine
        .compact(CompactInput {
            context: context.clone(),
            budget,
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
}
