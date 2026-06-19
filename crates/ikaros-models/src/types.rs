// SPDX-License-Identifier: GPL-3.0-only

use async_trait::async_trait;
use ikaros_core::{Result, redact_json, redact_secrets};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ModelToolCall>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
}

impl ModelMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ModelToolCall>,
    ) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            tool_calls,
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn tool_result(
        tool_call_id: impl Into<String>,
        tool_name: impl Into<String>,
        content: impl Into<String>,
    ) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name: Some(tool_name.into()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelRequest {
    pub messages: Vec<ModelMessage>,
    pub options: ModelRequestOptions,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ModelToolDefinition>,
}

impl ModelRequest {
    pub fn from_user_text(text: impl Into<String>) -> Self {
        Self {
            messages: vec![ModelMessage::user(text)],
            options: ModelRequestOptions::default(),
            tools: Vec::new(),
        }
    }

    pub fn with_options(mut self, options: ModelRequestOptions) -> Self {
        self.options = options;
        self
    }

    pub fn redacted(mut self) -> Self {
        for message in &mut self.messages {
            message.content = redact_secrets(&message.content);
            message.tool_calls = redact_model_tool_calls(std::mem::take(&mut message.tool_calls));
            message.tool_call_id = message
                .tool_call_id
                .take()
                .map(|tool_call_id| redact_secrets(&tool_call_id));
            message.tool_name = message
                .tool_name
                .take()
                .map(|tool_name| redact_secrets(&tool_name));
        }
        for tool in &mut self.tools {
            tool.name = redact_secrets(&tool.name);
            tool.description = redact_secrets(&tool.description);
            tool.input_schema = redact_json(tool.input_schema.clone());
        }
        self.options.stop = self
            .options
            .stop
            .into_iter()
            .map(|stop| redact_secrets(&stop))
            .collect();
        if !self.options.extra_body.is_empty() {
            let redacted = redact_json(serde_json::Value::Object(self.options.extra_body));
            self.options.extra_body = match redacted {
                serde_json::Value::Object(map) => map,
                _ => serde_json::Map::new(),
            };
        }
        self
    }

    pub fn estimated_tokens(&self) -> u32 {
        self.estimated_tokens_with_output_limit(self.options.max_tokens)
    }

    pub fn estimated_tokens_with_output_limit(&self, output_tokens: Option<u32>) -> u32 {
        let prompt_tokens = self
            .messages
            .iter()
            .map(|message| estimate_tokens(&message.content))
            .sum::<u32>();
        prompt_tokens.saturating_add(output_tokens.unwrap_or_default())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ModelRequestOptions {
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub top_p: Option<f32>,
    pub n: Option<u32>,
    pub presence_penalty: Option<f32>,
    pub frequency_penalty: Option<f32>,
    pub seed: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
    pub reasoning: ReasoningConfig,
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extra_body: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReasoningConfig {
    pub enabled: Option<bool>,
    pub effort: Option<ReasoningEffort>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
}

impl ReasoningEffort {
    pub fn as_wire_value(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TokenUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
}

impl TokenUsage {
    pub fn total_or_prompt_completion(&self) -> u32 {
        self.total_tokens.unwrap_or_else(|| {
            self.prompt_tokens
                .unwrap_or_default()
                .saturating_add(self.completion_tokens.unwrap_or_default())
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelContextProfile {
    pub context_window: u32,
    pub default_output_tokens: u32,
    pub tokenizer: ModelTokenizerKind,
    pub source: String,
}

impl ModelContextProfile {
    pub fn new(
        context_window: u32,
        default_output_tokens: u32,
        tokenizer: ModelTokenizerKind,
        source: impl Into<String>,
    ) -> Self {
        Self {
            context_window,
            default_output_tokens,
            tokenizer,
            source: source.into(),
        }
    }

    pub fn available_context_tokens(&self, reserved_system_tokens: u32) -> u32 {
        self.context_window
            .saturating_sub(self.default_output_tokens)
            .saturating_sub(reserved_system_tokens)
    }
}

impl Default for ModelContextProfile {
    fn default() -> Self {
        Self::new(
            128_000,
            4_096,
            ModelTokenizerKind::Heuristic,
            "model-provider-default",
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelTokenizerKind {
    #[default]
    Heuristic,
    OpenAiCompatible,
    Anthropic,
    Ollama,
    Mock,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelResponse {
    pub provider: String,
    pub model: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ModelToolCall>,
    pub usage: TokenUsage,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<ModelRequestDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelToolCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_arguments: Option<String>,
}

pub(crate) fn redact_model_tool_calls(calls: Vec<ModelToolCall>) -> Vec<ModelToolCall> {
    calls
        .into_iter()
        .map(|call| ModelToolCall {
            id: call.id.map(|id| redact_secrets(&id)),
            name: redact_secrets(&call.name),
            input: redact_json(call.input),
            raw_arguments: call
                .raw_arguments
                .map(|arguments| redact_secrets(&arguments)),
        })
        .collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelStream {
    pub provider: String,
    pub model: String,
    pub chunks: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ModelToolCall>,
    pub usage: TokenUsage,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<ModelStreamEvent>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<ModelRequestDiagnostic>,
}

impl ModelStream {
    pub fn content(&self) -> String {
        self.chunks.join("")
    }

    pub fn normalized_events(&self) -> Vec<ModelStreamEvent> {
        if !self.events.is_empty() {
            return self.events.clone();
        }
        let mut events = vec![ModelStreamEvent::Start {
            provider: self.provider.clone(),
            model: self.model.clone(),
        }];
        events.extend(
            self.chunks
                .iter()
                .filter(|chunk| !chunk.is_empty())
                .cloned()
                .map(ModelStreamEvent::TextDelta),
        );
        for call in &self.tool_calls {
            let id = call.id.clone().unwrap_or_else(|| call.name.clone());
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
        if self.usage.total_or_prompt_completion() > 0 {
            events.push(ModelStreamEvent::Usage(self.usage.clone()));
        }
        events.push(ModelStreamEvent::Done);
        events
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum ModelStreamEvent {
    Start { provider: String, model: String },
    TextDelta(String),
    ReasoningDelta(String),
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, args_delta: String },
    ToolCallEnd { id: String },
    RefusalDelta(String),
    Usage(TokenUsage),
    Error { message: String },
    Done,
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;
    fn estimate_request_tokens(&self, request: &ModelRequest) -> u32 {
        request.estimated_tokens()
    }
    fn context_profile(&self) -> ModelContextProfile {
        ModelContextProfile::default()
    }
    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse>;
    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let response = self.generate(request).await?;
        let mut stream = ModelStream {
            provider: response.provider,
            model: response.model,
            chunks: vec![response.content],
            tool_calls: response.tool_calls,
            usage: response.usage,
            events: Vec::new(),
            diagnostics: response.diagnostics,
        };
        stream.events = stream.normalized_events();
        Ok(stream)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRequestDiagnostic {
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
}

pub(crate) fn chunk_text(text: &str, max_chars: usize) -> Vec<String> {
    let max_chars = max_chars.max(1);
    let mut chunks = Vec::new();
    let mut current = String::new();
    let mut current_len = 0usize;
    for ch in text.chars() {
        current.push(ch);
        current_len += 1;
        if current_len >= max_chars {
            chunks.push(std::mem::take(&mut current));
            current_len = 0;
        }
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

pub(crate) fn estimate_tokens(text: &str) -> u32 {
    if text.trim().is_empty() {
        return 0;
    }
    let by_chars = (text.chars().count() as u32).saturating_add(3) / 4;
    let by_words = text.split_whitespace().count() as u32;
    by_chars.max(by_words).max(1)
}
