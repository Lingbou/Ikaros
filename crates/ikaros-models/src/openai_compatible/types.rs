// SPDX-License-Identifier: GPL-3.0-only

use crate::types::TokenUsage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub(super) struct OpenAiChatMessage {
    pub(super) role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) content: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) tool_calls: Vec<OpenAiOutboundToolCall>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_call_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct OpenAiOutboundToolCall {
    pub(super) id: String,
    pub(super) r#type: &'static str,
    pub(super) function: OpenAiOutboundFunctionCall,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct OpenAiOutboundFunctionCall {
    pub(super) name: String,
    pub(super) arguments: String,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct OpenAiToolDefinition {
    pub(super) r#type: &'static str,
    pub(super) function: OpenAiFunctionDefinition,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct OpenAiFunctionDefinition {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) parameters: serde_json::Value,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ChatCompletionResponse {
    pub(super) model: Option<String>,
    pub(super) choices: Vec<ChatChoice>,
    pub(super) usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ChatChoice {
    pub(super) message: ChatResponseMessage,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ChatResponseMessage {
    #[allow(dead_code)]
    pub(super) role: Option<String>,
    #[serde(default)]
    pub(super) content: Option<String>,
    #[serde(default)]
    pub(super) tool_calls: Vec<OpenAiToolCall>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct OpenAiToolCall {
    pub(super) id: Option<String>,
    #[allow(dead_code)]
    pub(super) r#type: Option<String>,
    pub(super) function: OpenAiFunctionCall,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct OpenAiFunctionCall {
    pub(super) name: String,
    #[serde(default)]
    pub(super) arguments: String,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ChatCompletionChunk {
    pub(super) model: Option<String>,
    pub(super) choices: Vec<ChatChunkChoice>,
    pub(super) usage: Option<TokenUsage>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ChatChunkChoice {
    pub(super) delta: ChatDelta,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct ChatDelta {
    pub(super) content: Option<String>,
    #[serde(default)]
    pub(super) reasoning: Option<String>,
    #[serde(default)]
    pub(super) reasoning_content: Option<String>,
    #[serde(default)]
    pub(super) refusal: Option<String>,
    #[serde(default)]
    pub(super) tool_calls: Vec<OpenAiStreamToolCallDelta>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct OpenAiStreamToolCallDelta {
    pub(super) index: Option<usize>,
    pub(super) id: Option<String>,
    pub(super) function: Option<OpenAiStreamFunctionDelta>,
}

#[derive(Debug, Clone, Deserialize)]
pub(super) struct OpenAiStreamFunctionDelta {
    pub(super) name: Option<String>,
    pub(super) arguments: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct OpenAiStreamToolCallAccumulator {
    pub(super) id: Option<String>,
    pub(super) name: String,
    pub(super) arguments: String,
}
