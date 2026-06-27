// SPDX-License-Identifier: GPL-3.0-only

use crate::http::{ModelHttpClient, ModelHttpRequest, ReqwestModelHttpClient};
use crate::transport::{ModelTransport, ModelTransportDescriptor, descriptor};
use crate::types::{
    ModelContentBlock, ModelContextProfile, ModelMessage, ModelProvider, ModelProviderCapabilities,
    ModelRequest, ModelRequestDiagnostic, ModelRequestOptions, ModelResponse, ModelStream,
    ModelStreamEvent, ModelTokenizerKind, ModelToolCall, ModelToolDefinition, TokenUsage,
};
use async_trait::async_trait;
use ikaros_core::{
    IkarosError, ModelConfig, RemoteProviderConfig, Result, redact_json, redact_secrets,
    resolve_config_value,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

const DEFAULT_OLLAMA_BASE_URL: &str = "http://127.0.0.1:11434";

#[derive(Clone)]
pub struct OllamaProvider {
    name: String,
    base_url: String,
    model: String,
    max_retries: u8,
    http: Arc<dyn ModelHttpClient>,
}

impl OllamaProvider {
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
            model: config.model.clone(),
            max_retries: config.max_retries,
            http,
        })
    }
}

impl ModelTransport for OllamaProvider {
    fn transport_descriptor(&self) -> ModelTransportDescriptor {
        descriptor(
            self.name.clone(),
            self.model.clone(),
            "harness-agent-loop",
            "ollama-chat",
            Some(self.base_url.clone()),
            true,
            true,
        )
    }
}

#[async_trait]
impl ModelProvider for OllamaProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn model_id(&self) -> &str {
        &self.model
    }

    fn context_profile(&self) -> ModelContextProfile {
        ModelContextProfile::new(
            ollama_context_window(&self.model),
            2_048,
            ModelTokenizerKind::Ollama,
            "ollama",
        )
    }

    fn capabilities(&self) -> ModelProviderCapabilities {
        ModelProviderCapabilities {
            chat: true,
            streaming: true,
            tool_calls: true,
            reasoning: false,
            json_mode: false,
            network: false,
            image_input: true,
            audio_input: false,
            file_input: false,
        }
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let request = request.redacted();
        let body = ollama_chat_request_body(&self.model, request, false);
        let url = format!("{}/api/chat", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result = self.http.send(ollama_http_post(&url, &body)?).await;
            match result {
                Ok(response) => {
                    let status = response.status;
                    let text = response.body;
                    if !(200..=299).contains(&status) {
                        last_error = Some(format!(
                            "Ollama model provider returned HTTP {status}: {}",
                            redact_secrets(&text)
                        ));
                        continue;
                    }
                    return parse_chat_response(&text, &self.name, &self.model);
                }
                Err(source) => {
                    last_error = Some(format!(
                        "Ollama model request failed on attempt {attempt}: {source}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(
            last_error.unwrap_or_else(|| "Ollama model request failed".into()),
        ))
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let request = request.redacted();
        let body = ollama_chat_request_body(&self.model, request, true);
        let url = format!("{}/api/chat", self.base_url);
        let mut last_error = None;
        for attempt in 0..=self.max_retries {
            let result = self.http.send(ollama_http_post(&url, &body)?).await;
            match result {
                Ok(response) => {
                    let status = response.status;
                    let text = response.body;
                    if !(200..=299).contains(&status) {
                        last_error = Some(format!(
                            "Ollama model provider returned HTTP {status}: {}",
                            redact_secrets(&text)
                        ));
                        continue;
                    }
                    match parse_stream_response(&text, &self.name, &self.model) {
                        Ok(stream) => return Ok(stream),
                        Err(error) => {
                            last_error = Some(format!(
                                "failed to parse Ollama model stream on attempt {attempt}: {error}"
                            ));
                        }
                    }
                }
                Err(source) => {
                    last_error = Some(format!(
                        "Ollama model stream request failed on attempt {attempt}: {source}"
                    ));
                }
            }
        }
        Err(IkarosError::Message(last_error.unwrap_or_else(|| {
            "Ollama model stream request failed".into()
        })))
    }
}

fn ollama_http_post(url: &str, body: &OllamaChatRequest) -> Result<ModelHttpRequest> {
    let mut headers = BTreeMap::new();
    headers.insert("content-type".into(), "application/json".into());
    Ok(ModelHttpRequest {
        method: "POST".into(),
        url: url.into(),
        headers,
        body: serde_json::to_string(body).map_err(|source| {
            IkarosError::Message(format!("failed to serialize Ollama request JSON: {source}"))
        })?,
    })
}

#[derive(Debug, Clone, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<OllamaToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaMessage {
    role: String,
    #[serde(default)]
    content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tool_calls: Vec<OllamaToolCall>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    images: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaToolCall {
    function: OllamaFunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OllamaFunctionCall {
    name: String,
    #[serde(default)]
    arguments: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
struct OllamaToolDefinition {
    r#type: &'static str,
    function: OllamaFunctionDefinition,
}

#[derive(Debug, Clone, Serialize)]
struct OllamaFunctionDefinition {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    seed: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    stop: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    num_predict: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
struct OllamaChatResponse {
    model: Option<String>,
    message: Option<OllamaMessage>,
    prompt_eval_count: Option<u32>,
    eval_count: Option<u32>,
}

fn ollama_chat_request_body(model: &str, request: ModelRequest, stream: bool) -> OllamaChatRequest {
    OllamaChatRequest {
        model: model.into(),
        messages: ollama_messages(request.messages),
        stream,
        tools: ollama_tools(request.tools),
        options: ollama_options(request.options),
    }
}

fn ollama_messages(messages: Vec<ModelMessage>) -> Vec<OllamaMessage> {
    messages
        .into_iter()
        .map(|message| {
            let content = redact_secrets(&ollama_message_text(&message));
            let images = ollama_message_images(&message);
            let tool_calls = if message.role == "assistant" {
                ollama_tool_calls(message.tool_calls)
            } else {
                Vec::new()
            };
            OllamaMessage {
                role: message.role,
                content,
                tool_calls,
                images,
                tool_name: message.tool_name.map(|name| redact_secrets(&name)),
            }
        })
        .collect()
}

fn ollama_message_text(message: &ModelMessage) -> String {
    if message.content_blocks.is_empty() {
        return message.content.clone();
    }
    let block_text = message
        .content_blocks
        .iter()
        .filter_map(|block| match block {
            ModelContentBlock::Text { text } => Some(text.clone()),
            ModelContentBlock::Image { image_url, .. } if !image_url.starts_with("data:image/") => {
                Some(format!("[image url omitted: {image_url}]"))
            }
            ModelContentBlock::Audio { audio_url, .. } => {
                Some(format!("[audio url omitted: {audio_url}]"))
            }
            ModelContentBlock::File { file_url, name, .. } => Some(format!(
                "[file url omitted: {} name={}]",
                file_url,
                name.as_deref().unwrap_or("unnamed")
            )),
            ModelContentBlock::ToolResult { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if message.content.trim().is_empty() || message.content == block_text {
        block_text
    } else if block_text.trim().is_empty() {
        message.content.clone()
    } else {
        format!("{}\n{}", message.content, block_text)
    }
}

fn ollama_message_images(message: &ModelMessage) -> Vec<String> {
    message
        .content_blocks
        .iter()
        .filter_map(|block| match block {
            ModelContentBlock::Image { image_url, .. } => {
                parse_data_url_image_base64(image_url).map(redact_secrets)
            }
            _ => None,
        })
        .collect()
}

fn parse_data_url_image_base64(image_url: &str) -> Option<&str> {
    let rest = image_url.strip_prefix("data:image/")?;
    let (_, data) = rest.split_once(";base64,")?;
    Some(data)
}

fn ollama_tool_calls(calls: Vec<ModelToolCall>) -> Vec<OllamaToolCall> {
    calls
        .into_iter()
        .map(|call| OllamaToolCall {
            function: OllamaFunctionCall {
                name: redact_secrets(&call.name),
                arguments: redact_json(call.input),
            },
        })
        .collect()
}

fn ollama_tools(tools: Vec<ModelToolDefinition>) -> Option<Vec<OllamaToolDefinition>> {
    (!tools.is_empty()).then(|| {
        tools
            .into_iter()
            .map(|tool| OllamaToolDefinition {
                r#type: "function",
                function: OllamaFunctionDefinition {
                    name: redact_secrets(&tool.name),
                    description: redact_secrets(&tool.description),
                    parameters: tool.input_schema,
                },
            })
            .collect()
    })
}

fn ollama_options(options: ModelRequestOptions) -> Option<OllamaOptions> {
    (options.temperature.is_some()
        || options.top_p.is_some()
        || options.seed.is_some()
        || !options.stop.is_empty()
        || options.max_tokens.is_some())
    .then_some(OllamaOptions {
        temperature: options.temperature,
        top_p: options.top_p,
        seed: options.seed,
        stop: options.stop,
        num_predict: options.max_tokens,
    })
}

pub(crate) fn parse_chat_response(
    text: &str,
    provider: &str,
    fallback_model: &str,
) -> Result<ModelResponse> {
    let parsed: OllamaChatResponse = serde_json::from_str(text).map_err(|source| {
        IkarosError::Message(format!(
            "failed to parse Ollama model response JSON: {source}"
        ))
    })?;
    let message = parsed.message.unwrap_or_else(|| OllamaMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: Vec::new(),
        images: Vec::new(),
        tool_name: None,
    });
    Ok(ModelResponse {
        provider: provider.into(),
        model: parsed.model.unwrap_or_else(|| fallback_model.into()),
        content: redact_secrets(&message.content),
        tool_calls: model_tool_calls(message.tool_calls),
        usage: TokenUsage {
            prompt_tokens: parsed.prompt_eval_count,
            completion_tokens: parsed.eval_count,
            total_tokens: token_total(parsed.prompt_eval_count, parsed.eval_count),
            ..TokenUsage::default()
        },
        diagnostics: Vec::new(),
    })
}

pub(crate) fn parse_stream_response(
    text: &str,
    provider: &str,
    fallback_model: &str,
) -> Result<ModelStream> {
    if text.trim_start().starts_with('{') && text.lines().count() <= 1 {
        let response = parse_chat_response(text, provider, fallback_model)?;
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
        return Ok(ModelStream {
            provider: response.provider,
            model: response.model,
            chunks,
            tool_calls: response.tool_calls,
            usage: response.usage,
            events,
            diagnostics: response
                .diagnostics
                .into_iter()
                .map(ModelRequestDiagnostic::sanitized)
                .collect(),
        });
    }

    let mut model = None;
    let mut chunks = Vec::new();
    let mut tool_calls = Vec::new();
    let mut usage = TokenUsage::default();
    let mut events = Vec::<ModelStreamEvent>::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parsed: OllamaChatResponse = serde_json::from_str(line).map_err(|source| {
            IkarosError::Message(format!(
                "failed to parse Ollama stream chunk JSON: {source}"
            ))
        })?;
        if model.is_none() {
            model = parsed.model.clone();
        }
        if parsed.prompt_eval_count.is_some() || parsed.eval_count.is_some() {
            usage = TokenUsage {
                prompt_tokens: parsed.prompt_eval_count,
                completion_tokens: parsed.eval_count,
                total_tokens: token_total(parsed.prompt_eval_count, parsed.eval_count),
                ..TokenUsage::default()
            };
        }
        if let Some(message) = parsed.message {
            let content = redact_secrets(&message.content);
            if !content.is_empty() {
                events.push(ModelStreamEvent::TextDelta(content.clone()));
                chunks.push(content);
            }
            let next_calls = model_tool_calls(message.tool_calls);
            for (offset, call) in next_calls.iter().enumerate() {
                let index = tool_calls.len() + offset;
                push_tool_call_events(&mut events, index, call);
            }
            tool_calls.extend(next_calls);
        }
    }

    if chunks.is_empty() && tool_calls.is_empty() {
        return Err(IkarosError::Message(
            "Ollama stream response did not contain content chunks".into(),
        ));
    }
    let model = model.unwrap_or_else(|| fallback_model.into());
    events.insert(
        0,
        ModelStreamEvent::Start {
            provider: provider.into(),
            model: model.clone(),
        },
    );
    if usage.total_or_prompt_completion() > 0 {
        events.push(ModelStreamEvent::Usage(usage.clone()));
    }
    events.push(ModelStreamEvent::Done);

    Ok(ModelStream {
        provider: provider.into(),
        model,
        chunks,
        tool_calls,
        usage,
        events,
        diagnostics: Vec::new(),
    })
}

fn model_tool_calls(calls: Vec<OllamaToolCall>) -> Vec<ModelToolCall> {
    calls
        .into_iter()
        .map(|call| {
            let input = redact_json(call.function.arguments);
            ModelToolCall {
                id: None,
                name: redact_secrets(&call.function.name),
                raw_arguments: Some(redact_secrets(&input.to_string())),
                input,
            }
        })
        .collect()
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
        push_tool_call_events(&mut events, index, call);
    }
    if usage.total_or_prompt_completion() > 0 {
        events.push(ModelStreamEvent::Usage(usage.clone()));
    }
    events.push(ModelStreamEvent::Done);
    events
}

fn push_tool_call_events(events: &mut Vec<ModelStreamEvent>, index: usize, call: &ModelToolCall) {
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

fn token_total(prompt: Option<u32>, completion: Option<u32>) -> Option<u32> {
    match (prompt, completion) {
        (Some(prompt), Some(completion)) => Some(prompt.saturating_add(completion)),
        _ => None,
    }
}

fn ollama_context_window(model: &str) -> u32 {
    let model = model.trim().to_ascii_lowercase();
    if model.contains("128k") || model.contains("128-k") {
        return 128_000;
    }
    if model.contains("64k") || model.contains("64-k") {
        return 64_000;
    }
    if model.contains("32k") || model.contains("32-k") {
        return 32_000;
    }
    if model.contains("16k") || model.contains("16-k") {
        return 16_000;
    }
    if model.contains("8k") || model.contains("8-k") {
        return 8_000;
    }
    8_192
}

fn provider_base_url(provider_settings: &RemoteProviderConfig) -> Result<String> {
    let configured = provider_settings.base_url.trim().trim_end_matches('/');
    if configured.is_empty() {
        Ok(DEFAULT_OLLAMA_BASE_URL.into())
    } else {
        resolve_config_value(
            configured,
            "providers.model.base_url for Ollama model provider",
        )
    }
}

#[cfg(test)]
pub(crate) fn test_chat_request_body(
    config: &ModelConfig,
    request: ModelRequest,
    stream: bool,
) -> serde_json::Value {
    let body = ollama_chat_request_body(&config.model, request.redacted(), stream);
    serde_json::to_value(body).expect("serialize Ollama chat request")
}
