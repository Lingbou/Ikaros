// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::RiskLevel;
use ikaros_harness::GuardrailConfig;
use ikaros_models::{ModelResponse, TokenUsage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentLoopInput {
    pub task_id: Option<String>,
    pub system_prompt: String,
    pub user_input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentLoopOptions {
    pub max_iterations: u32,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub stream: bool,
    pub guardrails: GuardrailConfig,
}

impl Default for AgentLoopOptions {
    fn default() -> Self {
        Self {
            max_iterations: 4,
            max_tokens: Some(512),
            temperature: Some(0.2),
            stream: false,
            guardrails: GuardrailConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AgentLoopStopReason {
    FinalAnswer,
    IterationBudget,
    PolicyDenied,
    WaitingForApproval,
    GuardrailHalt,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentLoopReport {
    pub stop_reason: AgentLoopStopReason,
    pub final_content: String,
    pub provider: String,
    pub model: String,
    pub usage: TokenUsage,
    pub streamed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stream_chunks: Vec<String>,
    pub iterations: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_call_diagnostics: Vec<AgentLoopToolCallDiagnostic>,
    pub tool_results: Vec<AgentLoopToolResult>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopToolCallParseStrategy {
    ProviderNativeToolCalls,
    DirectJsonObject,
    DirectJsonArray,
    FencedJson,
    EmbeddedJsonObject,
    EmbeddedJsonArray,
    PlainText,
}

impl AgentLoopToolCallParseStrategy {
    pub(super) fn is_repaired(self) -> bool {
        matches!(
            self,
            Self::DirectJsonArray
                | Self::FencedJson
                | Self::EmbeddedJsonObject
                | Self::EmbeddedJsonArray
        )
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ProviderNativeToolCalls => "provider_native_tool_calls",
            Self::DirectJsonObject => "direct_json_object",
            Self::DirectJsonArray => "direct_json_array",
            Self::FencedJson => "fenced_json",
            Self::EmbeddedJsonObject => "embedded_json_object",
            Self::EmbeddedJsonArray => "embedded_json_array",
            Self::PlainText => "plain_text",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentLoopToolCallDiagnostic {
    pub iteration: u32,
    pub strategy: AgentLoopToolCallParseStrategy,
    pub repaired: bool,
    pub native_tool_call_count: usize,
    pub tool_call_count: usize,
    pub has_final_answer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentLoopToolCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentLoopToolResult {
    pub iteration: u32,
    pub name: String,
    pub ok: bool,
    pub summary: String,
    pub output: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentLoopToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub risk: RiskLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct AgentLoopModelEnvelope {
    #[serde(default)]
    pub(super) final_answer: Option<String>,
    #[serde(default)]
    pub(super) tool_calls: Vec<AgentLoopToolCall>,
    #[serde(default)]
    pub(super) parse_strategy: Option<AgentLoopToolCallParseStrategy>,
}

pub(super) struct AgentLoopModelTurn {
    pub(super) response: ModelResponse,
    pub(super) streamed: bool,
    pub(super) stream_chunks: Vec<String>,
}

pub(super) struct AgentLoopFinish {
    pub(super) stop_reason: AgentLoopStopReason,
    pub(super) iterations: u32,
    pub(super) provider: String,
    pub(super) model: String,
    pub(super) usage: TokenUsage,
    pub(super) streamed: bool,
    pub(super) stream_chunks: Vec<String>,
    pub(super) final_content: String,
    pub(super) tool_call_diagnostics: Vec<AgentLoopToolCallDiagnostic>,
    pub(super) tool_results: Vec<AgentLoopToolResult>,
}
