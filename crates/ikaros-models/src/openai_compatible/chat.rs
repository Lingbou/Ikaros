// SPDX-License-Identifier: GPL-3.0-only

use super::{
    client::OpenAiCompatibleProvider,
    stream::parse_stream_response,
    tools::{model_tool_calls, openai_messages, openai_tools},
    types::{ChatCompletionRequest, ChatCompletionResponse},
};
use crate::types::{ModelProvider, ModelRequest, ModelResponse, ModelStream};
use async_trait::async_trait;
use ikaros_core::{IkarosError, Result, redact_secrets};

#[async_trait]
impl ModelProvider for OpenAiCompatibleProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let key = self.api_key()?;
        let request = request.redacted();
        let body = ChatCompletionRequest {
            model: self.model.clone(),
            messages: openai_messages(request.messages),
            max_tokens: request.max_tokens,
            temperature: self.compatible_temperature(request.temperature),
            tools: openai_tools(request.tools),
            tool_choice: None,
            stream: None,
        };
        let url = self.chat_completions_url();
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result = self
                .client
                .post(&url)
                .bearer_auth(&key)
                .json(&body)
                .send()
                .await;
            match result {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.map_err(|source| {
                        IkarosError::Message(format!("failed to read model response: {source}"))
                    })?;
                    if !status.is_success() {
                        last_error = Some(redacted_model_http_error(status, &text));
                        continue;
                    }
                    return parse_chat_completion_response(&text, &self.name, &self.model);
                }
                Err(source) => {
                    last_error = Some(format!(
                        "model request failed on attempt {attempt}: {source}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(
            last_error.unwrap_or_else(|| "model request failed".into()),
        ))
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let key = self.api_key()?;
        let request = request.redacted();
        let body = ChatCompletionRequest {
            model: self.model.clone(),
            messages: openai_messages(request.messages),
            max_tokens: request.max_tokens,
            temperature: self.compatible_temperature(request.temperature),
            tools: openai_tools(request.tools),
            tool_choice: None,
            stream: Some(true),
        };
        let url = self.chat_completions_url();
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result = self
                .client
                .post(&url)
                .bearer_auth(&key)
                .json(&body)
                .send()
                .await;
            match result {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.map_err(|source| {
                        IkarosError::Message(format!(
                            "failed to read model stream response: {source}"
                        ))
                    })?;
                    if !status.is_success() {
                        last_error = Some(redacted_model_http_error(status, &text));
                        continue;
                    }
                    match parse_stream_response(&text, &self.name, &self.model) {
                        Ok(stream) => return Ok(stream),
                        Err(error) => {
                            last_error = Some(format!(
                                "failed to parse model stream on attempt {attempt}: {error}"
                            ));
                        }
                    }
                }
                Err(source) => {
                    last_error = Some(format!(
                        "model stream request failed on attempt {attempt}: {source}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(
            last_error.unwrap_or_else(|| "model stream request failed".into()),
        ))
    }
}

pub(crate) fn redacted_model_http_error(status: reqwest::StatusCode, text: &str) -> String {
    format!(
        "model provider returned HTTP {status}: {}",
        redact_secrets(text)
    )
}

pub(crate) fn parse_chat_completion_response(
    text: &str,
    provider: &str,
    fallback_model: &str,
) -> Result<ModelResponse> {
    let parsed: ChatCompletionResponse = serde_json::from_str(text).map_err(|source| {
        IkarosError::Message(format!("failed to parse model response JSON: {source}"))
    })?;
    let content = parsed
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .unwrap_or_default();
    let tool_calls = parsed
        .choices
        .first()
        .map(|choice| model_tool_calls(&choice.message.tool_calls))
        .unwrap_or_default();
    Ok(ModelResponse {
        provider: provider.into(),
        model: parsed.model.unwrap_or_else(|| fallback_model.into()),
        content: redact_secrets(&content),
        tool_calls,
        usage: parsed.usage.unwrap_or_default(),
    })
}
