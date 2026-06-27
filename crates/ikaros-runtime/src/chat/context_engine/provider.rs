// SPDX-License-Identifier: GPL-3.0-only

use super::local::assemble_chat_context_with_engine;
use super::{
    CompactInput, CompactReport, ContextAssembleInput, ContextBundle, ContextEngine,
    compact_report_from_context_report,
};
use ikaros_context::{LlmSummaryCompressor, TrajectoryCompressor};
use ikaros_core::{IkarosError, Result};
use ikaros_models::{ModelMessage, ModelProvider, ModelRequest, ModelRequestOptions};
use std::{future::Future, pin::Pin};

pub struct ProviderSummaryContextEngine<'a> {
    provider: &'a dyn ModelProvider,
}

impl std::fmt::Debug for ProviderSummaryContextEngine<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderSummaryContextEngine")
            .field("provider", &self.provider.name())
            .finish()
    }
}

impl<'a> ProviderSummaryContextEngine<'a> {
    pub fn new(provider: &'a dyn ModelProvider) -> Self {
        Self { provider }
    }
}

impl ContextEngine for ProviderSummaryContextEngine<'_> {
    fn assemble<'a>(
        &'a self,
        input: ContextAssembleInput<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<ContextBundle>> + 'a>> {
        Box::pin(async move { assemble_chat_context_with_engine(self, input).await })
    }

    fn compact<'a>(
        &'a self,
        input: CompactInput,
    ) -> Pin<Box<dyn Future<Output = Result<CompactReport>> + 'a>> {
        Box::pin(async move {
            let estimator = input.tokenizer.estimator();
            let summary_request =
                LlmSummaryCompressor.prepare_summary_request(&input.context, input.budget.clone());
            let request = ModelRequest {
                messages: vec![
                    ModelMessage::system(summary_request.system_prompt),
                    ModelMessage::user(summary_request.user_prompt),
                ],
                options: ModelRequestOptions {
                    max_tokens: Some(summary_request.max_summary_tokens as u32),
                    temperature: Some(0.0),
                    ..ModelRequestOptions::default()
                },
                tools: Vec::new(),
            };
            let report = match self.provider.generate(request).await {
                Ok(response) => LlmSummaryCompressor
                    .compress_with_summary(
                        input.context.clone(),
                        input.budget.clone(),
                        &estimator,
                        response.content,
                    )
                    .map_err(|error| IkarosError::Message(error.to_string()))?,
                Err(_) => TrajectoryCompressor::default()
                    .compress(input.context.clone(), input.budget.clone(), &estimator)
                    .map_err(|error| IkarosError::Message(error.to_string()))?,
            };
            Ok(compact_report_from_context_report(report))
        })
    }
}
