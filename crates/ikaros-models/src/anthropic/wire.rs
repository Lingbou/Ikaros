// SPDX-License-Identifier: GPL-3.0-only

use crate::types::TokenUsage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub(super) struct AnthropicMessagesRequest {
    pub(super) model: String,
    pub(super) max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) system: Option<Vec<AnthropicContentBlock>>,
    pub(super) messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tools: Option<Vec<AnthropicToolDefinition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) thinking: Option<AnthropicThinking>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) output_config: Option<AnthropicOutputConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stream: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct AnthropicMessage {
    pub(super) role: String,
    pub(super) content: Vec<AnthropicContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AnthropicContentBlock {
    #[serde(rename = "type")]
    pub(super) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) input: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) source: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_use_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) is_error: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) cache_control: Option<AnthropicCacheControl>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AnthropicCacheControl {
    #[serde(rename = "type")]
    pub(super) kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct AnthropicToolDefinition {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct AnthropicThinking {
    #[serde(rename = "type")]
    pub(super) kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) display: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) budget_tokens: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct AnthropicOutputConfig {
    pub(super) effort: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AnthropicMessagesResponse {
    pub(super) model: Option<String>,
    pub(super) content: Vec<AnthropicContentBlock>,
    pub(super) usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub(super) struct AnthropicUsage {
    pub(super) input_tokens: Option<u32>,
    pub(super) output_tokens: Option<u32>,
    pub(super) cache_creation_input_tokens: Option<u32>,
    pub(super) cache_read_input_tokens: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AnthropicStreamEvent {
    #[serde(rename = "type")]
    pub(super) kind: String,
    pub(super) message: Option<AnthropicStreamMessage>,
    pub(super) content_block: Option<AnthropicContentBlock>,
    pub(super) delta: Option<AnthropicStreamDelta>,
    pub(super) usage: Option<AnthropicUsage>,
    pub(super) index: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AnthropicStreamMessage {
    pub(super) model: Option<String>,
    pub(super) usage: Option<AnthropicUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct AnthropicStreamDelta {
    #[serde(default, rename = "type")]
    pub(super) kind: Option<String>,
    pub(super) text: Option<String>,
    pub(super) partial_json: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct AnthropicStreamToolAccumulator {
    pub(super) id: Option<String>,
    pub(super) name: Option<String>,
    pub(super) arguments: String,
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
            cache_read_tokens: usage.cache_read_input_tokens,
            cache_write_tokens: usage.cache_creation_input_tokens,
        }
    }
}
