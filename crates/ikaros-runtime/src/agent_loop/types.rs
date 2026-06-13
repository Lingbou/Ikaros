// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{Result, RiskLevel};
use ikaros_harness::GuardrailConfig;
use ikaros_models::{ModelResponse, ModelStreamEvent, TokenUsage};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

pub type AgentEventId = String;
pub type AgentSessionId = String;
pub type AgentTurnId = String;

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentEvent {
    pub event_id: AgentEventId,
    pub session_id: AgentSessionId,
    pub turn_id: AgentTurnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_event_id: Option<AgentEventId>,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub source: AgentEventSource,
    pub kind: AgentEventKind,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

impl AgentEvent {
    pub fn new(
        session_id: impl Into<AgentSessionId>,
        turn_id: impl Into<AgentTurnId>,
        parent_event_id: Option<AgentEventId>,
        source: AgentEventSource,
        kind: AgentEventKind,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            event_id: Uuid::new_v4().to_string(),
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            parent_event_id,
            at: OffsetDateTime::now_utc(),
            source,
            kind,
            payload,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentEventSource {
    Runtime,
    User,
    Model,
    Tool,
    Harness,
    Context,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentEventKind {
    SessionStart,
    TurnStart,
    UserMessage,
    ModelStream(ModelStreamEvent),
    ToolStart,
    ToolUpdate,
    ToolEnd,
    ContextCompacted,
    ApprovalRequested,
    ApprovalResolved,
    TurnEnd,
    Error,
}

pub trait AgentEventSink: Send + Sync {
    fn emit(&self, event: &AgentEvent) -> Result<()>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoopAgentEventSink;

impl AgentEventSink for NoopAgentEventSink {
    fn emit(&self, _event: &AgentEvent) -> Result<()> {
        Ok(())
    }
}

static NOOP_AGENT_EVENT_SINK: NoopAgentEventSink = NoopAgentEventSink;

pub fn noop_agent_event_sink() -> &'static dyn AgentEventSink {
    &NOOP_AGENT_EVENT_SINK
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
