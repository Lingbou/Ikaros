// SPDX-License-Identifier: GPL-3.0-only

use crate::transport::{ModelTransport, ModelTransportDescriptor, descriptor};
use crate::types::{
    ModelMessage, ModelProvider, ModelRequest, ModelResponse, ModelStream, ModelStreamEvent,
    ModelToolCall, ModelToolDefinition, TokenUsage,
};
use async_trait::async_trait;
use ikaros_core::{
    IkarosError, ModelConfig, Result, redact_json, redact_secrets, resolve_config_secret,
    resolve_config_value,
};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::{mem, time::Duration};

const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Debug, Clone)]
pub struct AnthropicProvider {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    max_retries: u8,
    client: Client,
}

impl AnthropicProvider {
    pub fn from_config(provider_name: impl Into<String>, config: &ModelConfig) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|source| {
                IkarosError::Message(format!("failed to build Anthropic model client: {source}"))
            })?;
        Ok(Self {
            name: provider_name.into(),
            base_url: provider_base_url(config)?,
            model: resolve_config_value(&config.model, "model.default.model")?,
            api_key: config.api_key.clone(),
            max_retries: config.max_retries,
            client,
        })
    }

    fn api_key(&self) -> Result<String> {
        resolve_config_secret(&self.api_key, "providers.model.api_key")
    }
}

impl ModelTransport for AnthropicProvider {
    fn transport_descriptor(&self) -> ModelTransportDescriptor {
        descriptor(
            self.name.clone(),
            self.model.clone(),
            "harness-agent-loop",
            "anthropic-messages",
            Some(self.base_url.clone()),
            true,
            true,
        )
    }
}

#[async_trait]
impl ModelProvider for AnthropicProvider {
    fn name(&self) -> &str {
        &self.name
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let key = self.api_key()?;
        let request = request.redacted();
        let body = anthropic_messages_request_body(&self.model, request);
        let url = format!("{}/messages", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result = self
                .client
                .post(&url)
                .header("x-api-key", &key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .json(&body)
                .send()
                .await;
            match result {
                Ok(response) => {
                    let status = response.status();
                    let text = response.text().await.map_err(|source| {
                        IkarosError::Message(format!(
                            "failed to read Anthropic model response: {source}"
                        ))
                    })?;
                    if !status.is_success() {
                        last_error = Some(format!(
                            "Anthropic model provider returned HTTP {status}: {}",
                            redact_secrets(&text)
                        ));
                        continue;
                    }
                    return parse_messages_response(&text, &self.name, &self.model);
                }
                Err(source) => {
                    last_error = Some(format!(
                        "Anthropic model request failed on attempt {attempt}: {source}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(
            last_error.unwrap_or_else(|| "Anthropic model request failed".into()),
        ))
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let response = self.generate(request).await?;
        let chunks = (!response.content.is_empty())
            .then_some(response.content)
            .into_iter()
            .collect::<Vec<_>>();
        let events = model_stream_events_from_response(
            &response.provider,
            &response.model,
            &chunks,
            &response.tool_calls,
            &response.usage,
        );
        Ok(ModelStream {
            provider: response.provider,
            model: response.model,
            chunks,
            tool_calls: response.tool_calls,
            usage: response.usage,
            events,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicMessagesRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicToolDefinition>>,
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicMessage {
    role: String,
    content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AnthropicContentBlock {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    input: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicToolDefinition {
    name: String,
    description: String,
    input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
struct AnthropicMessagesResponse {
    model: Option<String>,
    content: Vec<AnthropicContentBlock>,
    usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct AnthropicUsage {
    input_tokens: Option<u32>,
    output_tokens: Option<u32>,
}

fn anthropic_messages_request_body(model: &str, request: ModelRequest) -> AnthropicMessagesRequest {
    let (system, messages) = anthropic_system_and_messages(request.messages);
    AnthropicMessagesRequest {
        model: model.into(),
        max_tokens: request.max_tokens.unwrap_or(512),
        temperature: request.temperature,
        system,
        messages,
        tools: anthropic_tools(request.tools),
    }
}

fn anthropic_system_and_messages(
    messages: Vec<ModelMessage>,
) -> (Option<String>, Vec<AnthropicMessage>) {
    let mut system = Vec::new();
    let mut anthropic_messages = Vec::new();
    let mut pending_tool_results = Vec::new();

    for message in messages {
        if message.role == "system" {
            if !message.content.trim().is_empty() {
                system.push(redact_secrets(&message.content));
            }
            continue;
        }

        if message.role == "tool" {
            if let Some(tool_call_id) = message.tool_call_id {
                pending_tool_results.push(anthropic_tool_result_block(
                    tool_call_id,
                    message.content,
                    false,
                ));
                continue;
            }
        }

        if !pending_tool_results.is_empty() {
            anthropic_messages.push(AnthropicMessage {
                role: "user".into(),
                content: mem::take(&mut pending_tool_results),
            });
        }

        match message.role.as_str() {
            "assistant" => anthropic_messages.push(AnthropicMessage {
                role: "assistant".into(),
                content: anthropic_assistant_content(message),
            }),
            "user" => anthropic_messages.push(AnthropicMessage {
                role: "user".into(),
                content: vec![anthropic_text_block(message.content)],
            }),
            _ => anthropic_messages.push(AnthropicMessage {
                role: "user".into(),
                content: vec![anthropic_text_block(message.content)],
            }),
        }
    }

    if !pending_tool_results.is_empty() {
        anthropic_messages.push(AnthropicMessage {
            role: "user".into(),
            content: pending_tool_results,
        });
    }

    let system = (!system.is_empty()).then(|| system.join("\n\n"));
    (system, anthropic_messages)
}

fn anthropic_assistant_content(message: ModelMessage) -> Vec<AnthropicContentBlock> {
    let mut content = Vec::new();
    if !message.content.trim().is_empty() {
        content.push(anthropic_text_block(message.content));
    }
    content.extend(message.tool_calls.into_iter().map(anthropic_tool_use_block));
    if content.is_empty() {
        content.push(anthropic_text_block(""));
    }
    content
}

fn anthropic_text_block(text: impl Into<String>) -> AnthropicContentBlock {
    AnthropicContentBlock {
        kind: "text".into(),
        text: Some(redact_secrets(&text.into())),
        id: None,
        name: None,
        input: None,
        tool_use_id: None,
        content: None,
        is_error: None,
    }
}

fn anthropic_tool_use_block(call: ModelToolCall) -> AnthropicContentBlock {
    AnthropicContentBlock {
        kind: "tool_use".into(),
        text: None,
        id: Some(redact_secrets(
            &call.id.unwrap_or_else(|| format!("toolu_{}", call.name)),
        )),
        name: Some(redact_secrets(&call.name)),
        input: Some(redact_json(call.input)),
        tool_use_id: None,
        content: None,
        is_error: None,
    }
}

fn anthropic_tool_result_block(
    tool_call_id: impl Into<String>,
    content: impl Into<String>,
    is_error: bool,
) -> AnthropicContentBlock {
    AnthropicContentBlock {
        kind: "tool_result".into(),
        text: None,
        id: None,
        name: None,
        input: None,
        tool_use_id: Some(redact_secrets(&tool_call_id.into())),
        content: Some(redact_secrets(&content.into())),
        is_error: is_error.then_some(true),
    }
}

fn anthropic_tools(tools: Vec<ModelToolDefinition>) -> Option<Vec<AnthropicToolDefinition>> {
    (!tools.is_empty()).then(|| {
        tools
            .into_iter()
            .map(|tool| AnthropicToolDefinition {
                name: redact_secrets(&tool.name),
                description: redact_secrets(&tool.description),
                input_schema: tool.input_schema,
            })
            .collect()
    })
}

pub(crate) fn parse_messages_response(
    text: &str,
    provider: &str,
    fallback_model: &str,
) -> Result<ModelResponse> {
    let parsed: AnthropicMessagesResponse = serde_json::from_str(text).map_err(|source| {
        IkarosError::Message(format!(
            "failed to parse Anthropic model response JSON: {source}"
        ))
    })?;
    let content = parsed
        .content
        .iter()
        .filter(|block| block.kind == "text")
        .filter_map(|block| block.text.as_deref())
        .map(redact_secrets)
        .collect::<Vec<_>>()
        .join("");
    let tool_calls = parsed
        .content
        .into_iter()
        .filter(|block| block.kind == "tool_use")
        .filter_map(anthropic_model_tool_call)
        .collect();
    Ok(ModelResponse {
        provider: provider.into(),
        model: parsed.model.unwrap_or_else(|| fallback_model.into()),
        content,
        tool_calls,
        usage: parsed.usage.unwrap_or_default().into(),
    })
}

fn anthropic_model_tool_call(block: AnthropicContentBlock) -> Option<ModelToolCall> {
    let name = block.name?;
    let input = block
        .input
        .map(redact_json)
        .unwrap_or_else(|| serde_json::Value::Object(Default::default()));
    Some(ModelToolCall {
        id: block.id.map(|id| redact_secrets(&id)),
        name: redact_secrets(&name),
        raw_arguments: Some(redact_secrets(&input.to_string())),
        input,
    })
}

fn model_stream_events_from_response(
    provider: &str,
    model: &str,
    chunks: &[String],
    tool_calls: &[ModelToolCall],
    usage: &TokenUsage,
) -> Vec<ModelStreamEvent> {
    let mut events = vec![ModelStreamEvent::Start {
        provider: provider.into(),
        model: model.into(),
    }];
    events.extend(
        chunks
            .iter()
            .filter(|chunk| !chunk.is_empty())
            .cloned()
            .map(ModelStreamEvent::TextDelta),
    );
    for (index, call) in tool_calls.iter().enumerate() {
        let id = call
            .id
            .clone()
            .unwrap_or_else(|| format!("tool_call_{index}"));
        events.push(ModelStreamEvent::ToolCallStart {
            id: id.clone(),
            name: call.name.clone(),
        });
        if let Some(arguments) = &call.raw_arguments {
            events.push(ModelStreamEvent::ToolCallDelta {
                id: id.clone(),
                args_delta: arguments.clone(),
            });
        }
        events.push(ModelStreamEvent::ToolCallEnd { id });
    }
    if usage.total_or_prompt_completion() > 0 {
        events.push(ModelStreamEvent::Usage(usage.clone()));
    }
    events.push(ModelStreamEvent::Done);
    events
}

impl From<AnthropicUsage> for TokenUsage {
    fn from(usage: AnthropicUsage) -> Self {
        TokenUsage {
            prompt_tokens: usage.input_tokens,
            completion_tokens: usage.output_tokens,
            total_tokens: match (usage.input_tokens, usage.output_tokens) {
                (Some(input), Some(output)) => Some(input.saturating_add(output)),
                _ => None,
            },
        }
    }
}

fn provider_base_url(config: &ModelConfig) -> Result<String> {
    Ok(resolve_config_value(
        &config.base_url,
        "providers.model.base_url for Anthropic-compatible model provider",
    )?
    .trim_end_matches('/')
    .into())
}

#[cfg(test)]
pub(crate) fn test_messages_request_body(
    config: &ModelConfig,
    request: ModelRequest,
) -> serde_json::Value {
    let body = anthropic_messages_request_body(&config.model, request.redacted());
    serde_json::to_value(body).expect("serialize Anthropic messages request")
}
