// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::RiskLevel;
use ikaros_harness::{GuardrailConfig, ToolExecutionMode};
use ikaros_models::{ModelRequestOptions, ModelResponse, ModelStreamEvent, TokenUsage};
pub use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentSessionId, AgentTurnId,
    noop_agent_event_sink,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentLoopInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub system_prompt: String,
    pub user_input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentLoopOptions {
    pub max_iterations: u32,
    pub request_options: ModelRequestOptions,
    pub stream: bool,
    pub guardrails: GuardrailConfig,
}

impl Default for AgentLoopOptions {
    fn default() -> Self {
        Self {
            max_iterations: 4,
            request_options: ModelRequestOptions::default(),
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
    Cancelled,
    ProviderError,
    Compacted,
    ToolError,
    ContextLimit,
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<AgentEvent>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentLoopToolCallParseStrategy {
    ProviderNativeToolCalls,
    JsonFallback,
    PlainText,
}

impl AgentLoopToolCallParseStrategy {
    pub(super) fn is_repaired(self) -> bool {
        false
    }

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ProviderNativeToolCalls => "provider_native_tool_calls",
            Self::JsonFallback => "json_fallback",
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
    pub execution_mode: ToolExecutionMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
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
    pub(super) stream_events: Vec<ModelStreamEvent>,
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
    pub(super) events: Vec<AgentEvent>,
}
