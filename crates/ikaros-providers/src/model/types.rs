// SPDX-License-Identifier: GPL-3.0-only

use async_trait::async_trait;
use ikaros_core::{Result, redact_json, redact_secrets};
pub use ikaros_protocol::{ModelRequestDiagnostic, ModelStreamEvent, TokenUsage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_blocks: Vec<ModelContentBlock>,
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
            content_blocks: Vec::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            content_blocks: Vec::new(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            content_blocks: Vec::new(),
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
            content_blocks: Vec::new(),
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
            content_blocks: Vec::new(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
            tool_name: Some(tool_name.into()),
        }
    }

    pub fn user_with_content_blocks(blocks: Vec<ModelContentBlock>) -> Self {
        Self {
            role: "user".into(),
            content: content_blocks_text(&blocks),
            content_blocks: blocks,
            tool_calls: Vec::new(),
            tool_call_id: None,
            tool_name: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelContentBlock {
    Text {
        text: String,
    },
    Image {
        image_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    Audio {
        audio_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
    },
    File {
        file_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    ToolResult {
        tool_call_id: String,
        text: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

impl ModelContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text { text: text.into() }
    }

    pub fn image_url(url: impl Into<String>) -> Self {
        Self::Image {
            image_url: url.into(),
            mime_type: None,
            detail: None,
        }
    }

    pub fn redacted(self) -> Self {
        match self {
            Self::Text { text } => Self::Text {
                text: redact_secrets(&text),
            },
            Self::Image {
                image_url,
                mime_type,
                detail,
            } => Self::Image {
                image_url: redact_secrets(&image_url),
                mime_type: mime_type.map(|value| redact_secrets(&value)),
                detail: detail.map(|value| redact_secrets(&value)),
            },
            Self::Audio {
                audio_url,
                mime_type,
            } => Self::Audio {
                audio_url: redact_secrets(&audio_url),
                mime_type: mime_type.map(|value| redact_secrets(&value)),
            },
            Self::File {
                file_url,
                mime_type,
                name,
            } => Self::File {
                file_url: redact_secrets(&file_url),
                mime_type: mime_type.map(|value| redact_secrets(&value)),
                name: name.map(|value| redact_secrets(&value)),
            },
            Self::ToolResult {
                tool_call_id,
                text,
                is_error,
            } => Self::ToolResult {
                tool_call_id: redact_secrets(&tool_call_id),
                text: redact_secrets(&text),
                is_error,
            },
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
            message.content_blocks = std::mem::take(&mut message.content_blocks)
                .into_iter()
                .map(ModelContentBlock::redacted)
                .collect();
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
            .map(|message| {
                let blocks_tokens = message
                    .content_blocks
                    .iter()
                    .map(estimate_content_block_tokens)
                    .sum::<u32>();
                let text_tokens = if !message.content_blocks.is_empty()
                    && message.content == content_blocks_text(&message.content_blocks)
                {
                    0
                } else {
                    estimate_tokens(&message.content)
                };
                text_tokens.saturating_add(blocks_tokens)
            })
            .sum::<u32>();
        prompt_tokens.saturating_add(output_tokens.unwrap_or_default())
    }
}

fn content_blocks_text(blocks: &[ModelContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ModelContentBlock::Text { text } => Some(text.as_str()),
            ModelContentBlock::ToolResult { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn estimate_content_block_tokens(block: &ModelContentBlock) -> u32 {
    match block {
        ModelContentBlock::Text { text } | ModelContentBlock::ToolResult { text, .. } => {
            estimate_tokens(text)
        }
        ModelContentBlock::Image { .. } => 1_000,
        ModelContentBlock::Audio { .. } => 2_000,
        ModelContentBlock::File { name, .. } => name.as_deref().map_or(1_000, estimate_tokens),
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelProviderCapabilities {
    pub chat: bool,
    pub streaming: bool,
    pub tool_calls: bool,
    pub reasoning: bool,
    pub json_mode: bool,
    pub network: bool,
    #[serde(default)]
    pub image_input: bool,
    #[serde(default)]
    pub audio_input: bool,
    #[serde(default)]
    pub file_input: bool,
}

impl ModelProviderCapabilities {
    pub fn text_only() -> Self {
        Self {
            chat: true,
            streaming: false,
            tool_calls: false,
            reasoning: false,
            json_mode: false,
            network: false,
            image_input: false,
            audio_input: false,
            file_input: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelProviderCost {
    pub currency: String,
    pub input_per_million: Option<f64>,
    pub output_per_million: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_per_million: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_per_million: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorKind {
    Auth,
    RateLimited,
    Transient,
    BadRequest,
    ContextLimit,
    Network,
    Unknown,
}

impl ProviderErrorKind {
    pub fn classify_status(status: u16) -> Self {
        match status {
            401 | 403 => Self::Auth,
            408 | 409 | 425 | 429 => Self::RateLimited,
            500..=599 => Self::Transient,
            400 | 404 | 422 => Self::BadRequest,
            _ => Self::Unknown,
        }
    }

    pub fn retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Transient | Self::Network)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealthStatus {
    Unknown,
    Healthy,
    Degraded,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderHealthState {
    pub provider: String,
    pub model: String,
    pub status: ProviderHealthStatus,
    pub consecutive_failures: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error_kind: Option<ProviderErrorKind>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub last_error_summary: String,
}

impl ProviderHealthState {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
            status: ProviderHealthStatus::Unknown,
            consecutive_failures: 0,
            last_error_kind: None,
            last_error_summary: String::new(),
        }
    }

    pub fn record_success(&mut self) {
        self.status = ProviderHealthStatus::Healthy;
        self.consecutive_failures = 0;
        self.last_error_kind = None;
        self.last_error_summary.clear();
    }

    pub fn record_failure(&mut self, kind: ProviderErrorKind, summary: impl Into<String>) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.last_error_kind = Some(kind);
        self.last_error_summary = redact_secrets(&summary.into());
        self.status = if self.consecutive_failures >= 3 {
            ProviderHealthStatus::Unavailable
        } else {
            ProviderHealthStatus::Degraded
        };
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelProviderDescriptor {
    pub provider: String,
    pub model: String,
    pub profile: String,
    pub profile_policy: ModelProviderProfilePolicy,
    pub capabilities: ModelProviderCapabilities,
    pub context: ModelContextProfile,
    pub cost: ModelProviderCost,
    pub health: ProviderHealthState,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelProviderProfileCatalogEntry {
    pub provider: String,
    pub profile: String,
    pub profile_policy: ModelProviderProfilePolicy,
    pub capabilities: ModelProviderCapabilities,
    pub context: ModelContextProfile,
    pub auto_base_url_markers: Vec<String>,
    pub auto_model_markers: Vec<String>,
    pub auto_model_tail_prefixes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelProviderProfilePolicy {
    pub temperature: String,
    pub reasoning: String,
    pub message: String,
    pub tool_schema: String,
    pub request_body: String,
    #[serde(default = "default_prompt_cache_policy")]
    pub prompt_cache: String,
    pub retry_without_parameters: Vec<String>,
}

fn default_prompt_cache_policy() -> String {
    "none".into()
}

impl ModelProviderProfilePolicy {
    pub fn native(profile: impl Into<String>) -> Self {
        let profile = profile.into();
        Self {
            temperature: profile.clone(),
            reasoning: profile.clone(),
            message: profile.clone(),
            tool_schema: profile.clone(),
            request_body: profile,
            prompt_cache: "none".into(),
            retry_without_parameters: Vec::new(),
        }
    }

    pub fn with_prompt_cache(mut self, prompt_cache: impl Into<String>) -> Self {
        self.prompt_cache = prompt_cache.into();
        self
    }
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

pub trait ModelStreamEventSink: Send {
    fn emit(&mut self, event: ModelStreamEvent) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct NoopModelStreamEventSink;

impl ModelStreamEventSink for NoopModelStreamEventSink {
    fn emit(&mut self, _event: ModelStreamEvent) -> Result<()> {
        Ok(())
    }
}

#[async_trait]
pub trait ModelProvider: Send + Sync {
    fn name(&self) -> &str;
    fn model_id(&self) -> &str {
        ""
    }
    fn estimate_request_tokens(&self, request: &ModelRequest) -> u32 {
        request.estimated_tokens()
    }
    fn context_profile(&self) -> ModelContextProfile {
        ModelContextProfile::default()
    }
    fn capabilities(&self) -> ModelProviderCapabilities {
        ModelProviderCapabilities::text_only()
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
            diagnostics: response
                .diagnostics
                .into_iter()
                .map(ModelRequestDiagnostic::sanitized)
                .collect(),
        };
        stream.events = stream.normalized_events();
        Ok(stream)
    }
    async fn stream_with_events(
        &self,
        request: ModelRequest,
        event_sink: &mut dyn ModelStreamEventSink,
    ) -> Result<ModelStream> {
        let stream = self.stream(request).await?;
        for event in stream.normalized_events() {
            event_sink.emit(event)?;
        }
        Ok(stream)
    }
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
