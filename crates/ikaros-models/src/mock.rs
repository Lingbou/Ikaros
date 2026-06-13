// SPDX-License-Identifier: GPL-3.0-only

use crate::transport::{ModelTransport, ModelTransportDescriptor, descriptor};
use crate::types::{
    ModelProvider, ModelRequest, ModelResponse, ModelStream, TokenUsage, chunk_text,
    estimate_tokens,
};
use async_trait::async_trait;
use ikaros_core::{Result, redact_secrets};

#[derive(Debug, Clone)]
pub struct MockModelProvider {
    model: String,
}

impl Default for MockModelProvider {
    fn default() -> Self {
        Self {
            model: "mock-ikaros".into(),
        }
    }
}

impl MockModelProvider {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
        }
    }
}

impl ModelTransport for MockModelProvider {
    fn transport_descriptor(&self) -> ModelTransportDescriptor {
        descriptor(
            "mock",
            self.model.clone(),
            "harness-agent-loop",
            "mock",
            None,
            true,
            false,
        )
    }
}

#[async_trait]
impl ModelProvider for MockModelProvider {
    fn name(&self) -> &str {
        "mock"
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let last = request
            .messages
            .last()
            .map(|message| redact_secrets(&message.content))
            .unwrap_or_else(|| "empty request".into());
        let content = if last.contains("Heuristic review report:")
            && last.contains("Guarded Patch Iteration")
        {
            "Mock model review:\nResidual Risks: inspect the heuristic findings and changed files for behavior gaps.\nFocused Tests: run the narrowest affected test/check command, then workspace checks if risk remains.\nGuarded Patch Iteration: address high-severity findings first, regenerate the diff through guarded edit approval, and rerun review.".into()
        } else {
            format!(
                "Mock Ikaros plan: acknowledge task, evaluate policy, execute safe skills, audit result. Input: {last}"
            )
        };
        Ok(ModelResponse {
            provider: self.name().into(),
            model: self.model.clone(),
            content: content.clone(),
            tool_calls: Vec::new(),
            usage: TokenUsage {
                prompt_tokens: Some(last.split_whitespace().count() as u32),
                completion_tokens: Some(estimate_tokens(&content)),
                total_tokens: None,
            },
        })
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let response = self.generate(request).await?;
        Ok(ModelStream {
            provider: response.provider,
            model: response.model,
            chunks: chunk_text(&response.content, 48),
            tool_calls: response.tool_calls,
            usage: response.usage,
            events: Vec::new(),
        })
    }
}
