// SPDX-License-Identifier: GPL-3.0-only

use crate::http::{ModelHttpClient, ModelHttpRequest, ReqwestModelHttpClient};
use crate::transport::{ModelTransport, ModelTransportDescriptor, descriptor};
use crate::types::{
    ModelContextProfile, ModelMessage, ModelProvider, ModelRequest, ModelResponse, ModelStream,
    ModelStreamEvent, ModelTokenizerKind, ModelToolCall, ModelToolDefinition, ReasoningEffort,
    TokenUsage,
};
use async_trait::async_trait;
use ikaros_core::{
    IkarosError, ModelConfig, RemoteProviderConfig, Result, redact_json, redact_secrets,
    resolve_config_secret, resolve_config_value,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, mem, sync::Arc, time::Duration};

const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_DEFAULT_OUTPUT_LIMIT: u32 = 128_000;
const LEGACY_MANUAL_THINKING_CLAUDE_SUBSTRINGS: &[&str] = &[
    "claude-3",
    "claude-opus-4-0",
    "claude-opus-4.0",
    "claude-opus-4-1",
    "claude-opus-4.1",
    "claude-sonnet-4-0",
    "claude-sonnet-4.0",
    "claude-opus-4-2025",
    "claude-sonnet-4-2025",
    "claude-opus-4-5",
    "claude-opus-4.5",
    "claude-sonnet-4-5",
    "claude-sonnet-4.5",
    "claude-haiku-4-5",
    "claude-haiku-4.5",
];
const NO_XHIGH_CLAUDE_SUBSTRINGS: &[&str] = &[
    "claude-opus-4-6",
    "claude-opus-4.6",
    "claude-sonnet-4-6",
    "claude-sonnet-4.6",
];

#[derive(Clone)]
pub struct AnthropicProvider {
    name: String,
    base_url: String,
    model: String,
    api_key: String,
    max_retries: u8,
    http: Arc<dyn ModelHttpClient>,
}

impl AnthropicProvider {
    pub fn from_config(
        provider_name: impl Into<String>,
        config: &ModelConfig,
        provider_settings: &RemoteProviderConfig,
    ) -> Result<Self> {
        Self::from_config_with_http_client(
            provider_name,
            config,
            provider_settings,
            Arc::new(ReqwestModelHttpClient::new(Duration::from_millis(
                config.timeout_ms,
            ))?),
        )
    }

    pub fn from_config_with_http_client(
        provider_name: impl Into<String>,
        config: &ModelConfig,
        provider_settings: &RemoteProviderConfig,
        http: Arc<dyn ModelHttpClient>,
    ) -> Result<Self> {
        Ok(Self {
            name: provider_name.into(),
            base_url: provider_base_url(provider_settings)?,
            model: resolve_config_value(&config.model, "model.default.model")?,
            api_key: provider_settings.api_key.clone(),
            max_retries: config.max_retries,
            http,
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

    fn model_id(&self) -> &str {
        &self.model
    }

    fn context_profile(&self) -> ModelContextProfile {
        ModelContextProfile::new(
            anthropic_context_window(&self.model),
            anthropic_default_max_tokens(&self.model),
            ModelTokenizerKind::Anthropic,
            "anthropic",
        )
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let key = self.api_key()?;
        let request = request.redacted();
        let body = anthropic_messages_request_body(&self.model, request);
        let url = format!("{}/messages", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result = self
                .http
                .send(anthropic_http_post(&url, &key, &body)?)
                .await;
            match result {
                Ok(response) => {
                    let status = response.status;
                    let text = response.body;
                    if !(200..=299).contains(&status) {
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
            diagnostics: response.diagnostics,
        })
    }
}

fn anthropic_http_post(
    url: &str,
    key: &str,
    body: &AnthropicMessagesRequest,
) -> Result<ModelHttpRequest> {
    let mut headers = BTreeMap::new();
    headers.insert("x-api-key".into(), key.into());
    headers.insert("anthropic-version".into(), ANTHROPIC_VERSION.into());
    headers.insert("content-type".into(), "application/json".into());
    Ok(ModelHttpRequest {
        method: "POST".into(),
        url: url.into(),
        headers,
        body: serde_json::to_string(body).map_err(|source| {
            IkarosError::Message(format!(
                "failed to serialize Anthropic request JSON: {source}"
            ))
        })?,
    })
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicMessagesRequest {
    model: String,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<AnthropicToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<AnthropicThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output_config: Option<AnthropicOutputConfig>,
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

#[derive(Debug, Clone, Serialize)]
struct AnthropicThinking {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
struct AnthropicOutputConfig {
    effort: String,
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
    let mut max_tokens = request
        .options
        .max_tokens
        .unwrap_or_else(|| anthropic_default_max_tokens(model));
    let mut temperature = request.options.temperature;
    let mut top_p = request.options.top_p;
    let (thinking, output_config) = anthropic_thinking_fields(
        model,
        &request.options.reasoning,
        &mut max_tokens,
        &mut temperature,
    );
    if forbids_sampling_params(model) {
        temperature = None;
        top_p = None;
    }
    AnthropicMessagesRequest {
        model: model.into(),
        max_tokens,
        temperature,
        top_p,
        system,
        messages,
        tools: anthropic_tools(request.tools),
        thinking,
        output_config,
    }
}

fn anthropic_thinking_fields(
    model: &str,
    reasoning: &crate::types::ReasoningConfig,
    max_tokens: &mut u32,
    temperature: &mut Option<f32>,
) -> (Option<AnthropicThinking>, Option<AnthropicOutputConfig>) {
    let configured = reasoning.enabled.is_some() || reasoning.effort.is_some();
    if !configured
        || reasoning.enabled == Some(false)
        || matches!(reasoning.effort, Some(ReasoningEffort::None))
        || model.to_ascii_lowercase().contains("haiku")
    {
        return (None, None);
    }

    let effort = reasoning.effort.unwrap_or(ReasoningEffort::Medium);
    if supports_adaptive_thinking(model) {
        let mut adaptive_effort = anthropic_adaptive_effort(effort);
        if adaptive_effort == "xhigh" && !supports_xhigh_effort(model) {
            adaptive_effort = "max";
        }
        return (
            Some(AnthropicThinking {
                kind: "adaptive".into(),
                display: Some("summarized".into()),
                budget_tokens: None,
            }),
            Some(AnthropicOutputConfig {
                effort: adaptive_effort.into(),
            }),
        );
    }

    let budget = anthropic_manual_thinking_budget(effort);
    *temperature = Some(1.0);
    *max_tokens = (*max_tokens).max(budget.saturating_add(4096));
    (
        Some(AnthropicThinking {
            kind: "enabled".into(),
            display: None,
            budget_tokens: Some(budget),
        }),
        None,
    )
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
        diagnostics: Vec::new(),
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

#[cfg(test)]
pub(crate) fn test_model_stream_events_from_response(
    provider: &str,
    model: &str,
    chunks: &[String],
    tool_calls: &[ModelToolCall],
    usage: &TokenUsage,
) -> Vec<ModelStreamEvent> {
    model_stream_events_from_response(provider, model, chunks, tool_calls, usage)
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

fn anthropic_default_max_tokens(model: &str) -> u32 {
    let model = model.to_ascii_lowercase().replace('.', "-");
    let limits = [
        ("claude-fable", 128_000),
        ("claude-opus-4-8", 128_000),
        ("claude-opus-4-7", 128_000),
        ("claude-opus-4-6", 128_000),
        ("claude-sonnet-4-6", 64_000),
        ("claude-opus-4-5", 64_000),
        ("claude-sonnet-4-5", 64_000),
        ("claude-haiku-4-5", 64_000),
        ("claude-opus-4", 32_000),
        ("claude-sonnet-4", 64_000),
        ("claude-3-7-sonnet", 128_000),
        ("claude-3-5-sonnet", 8_192),
        ("claude-3-5-haiku", 8_192),
        ("claude-3-opus", 4_096),
        ("claude-3-sonnet", 4_096),
        ("claude-3-haiku", 4_096),
        ("minimax", 131_072),
        ("qwen3", 65_536),
    ];
    limits
        .iter()
        .filter(|(needle, _)| model.contains(needle))
        .max_by_key(|(needle, _)| needle.len())
        .map(|(_, limit)| *limit)
        .unwrap_or(ANTHROPIC_DEFAULT_OUTPUT_LIMIT)
}

fn anthropic_context_window(model: &str) -> u32 {
    let model = model.to_ascii_lowercase().replace('.', "-");
    if model.contains("claude") {
        return 200_000;
    }
    if model.contains("minimax") {
        return 1_000_000;
    }
    if model.contains("qwen3") {
        return 128_000;
    }
    200_000
}

fn supports_adaptive_thinking(model: &str) -> bool {
    if !is_claude_model(model) {
        return false;
    }
    !LEGACY_MANUAL_THINKING_CLAUDE_SUBSTRINGS
        .iter()
        .any(|needle| model.to_ascii_lowercase().contains(needle))
}

fn supports_xhigh_effort(model: &str) -> bool {
    supports_adaptive_thinking(model)
        && !NO_XHIGH_CLAUDE_SUBSTRINGS
            .iter()
            .any(|needle| model.to_ascii_lowercase().contains(needle))
}

fn forbids_sampling_params(model: &str) -> bool {
    if !is_claude_model(model) {
        return false;
    }
    let model = model.to_ascii_lowercase();
    if NO_XHIGH_CLAUDE_SUBSTRINGS
        .iter()
        .any(|needle| model.contains(needle))
    {
        return false;
    }
    !LEGACY_MANUAL_THINKING_CLAUDE_SUBSTRINGS
        .iter()
        .any(|needle| model.contains(needle))
}

fn is_claude_model(model: &str) -> bool {
    model.to_ascii_lowercase().contains("claude")
}

fn anthropic_adaptive_effort(effort: ReasoningEffort) -> &'static str {
    match effort {
        ReasoningEffort::None => "medium",
        ReasoningEffort::Minimal | ReasoningEffort::Low => "low",
        ReasoningEffort::Medium => "medium",
        ReasoningEffort::High => "high",
        ReasoningEffort::XHigh => "xhigh",
        ReasoningEffort::Max => "max",
    }
}

fn anthropic_manual_thinking_budget(effort: ReasoningEffort) -> u32 {
    match effort {
        ReasoningEffort::Low | ReasoningEffort::Minimal => 4_000,
        ReasoningEffort::High => 16_000,
        ReasoningEffort::XHigh | ReasoningEffort::Max => 32_000,
        ReasoningEffort::None | ReasoningEffort::Medium => 8_000,
    }
}

fn provider_base_url(provider_settings: &RemoteProviderConfig) -> Result<String> {
    Ok(resolve_config_value(
        &provider_settings.base_url,
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
