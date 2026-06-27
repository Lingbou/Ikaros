// SPDX-License-Identifier: GPL-3.0-only
//! Stable protocol types shared by CLI, TUI, gateway, API, and replay surfaces.
//!
//! This crate is intentionally small and domain-facing. It contains the durable
//! wire shapes that product surfaces should exchange, not provider adapters,
//! stores, or runtime implementation details.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Write as _;
use time::OffsetDateTime;

pub const IKAROS_PROTOCOL_VERSION: u32 = 1;
pub const IKAROS_PROTOCOL_NAME: &str = "ikaros-protocol";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WireEnvelope<T> {
    pub protocol: String,
    pub version: u32,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub kind: String,
    pub data: T,
}

impl<T> WireEnvelope<T> {
    pub fn new(kind: impl Into<String>, data: T) -> Self {
        Self {
            protocol: IKAROS_PROTOCOL_NAME.into(),
            version: IKAROS_PROTOCOL_VERSION,
            at: OffsetDateTime::now_utc(),
            kind: kind.into(),
            data,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct TokenUsage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_tokens: Option<u32>,
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

impl<'de> Deserialize<'de> for TokenUsage {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize, Default)]
        struct TokenUsageWire {
            prompt_tokens: Option<u32>,
            completion_tokens: Option<u32>,
            total_tokens: Option<u32>,
            #[serde(default, alias = "cache_read_input_tokens")]
            cache_read_tokens: Option<u32>,
            #[serde(default, alias = "cache_creation_input_tokens")]
            cache_write_tokens: Option<u32>,
            #[serde(default)]
            prompt_tokens_details: Option<PromptTokensDetailsWire>,
        }

        #[derive(Deserialize, Default)]
        struct PromptTokensDetailsWire {
            cached_tokens: Option<u32>,
        }

        let wire = TokenUsageWire::deserialize(deserializer)?;
        Ok(Self {
            prompt_tokens: wire.prompt_tokens,
            completion_tokens: wire.completion_tokens,
            total_tokens: wire.total_tokens,
            cache_read_tokens: wire.cache_read_tokens.or_else(|| {
                wire.prompt_tokens_details
                    .and_then(|details| details.cached_tokens)
            }),
            cache_write_tokens: wire.cache_write_tokens,
        })
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelRequestDiagnostic {
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameter: Option<String>,
}

pub const MODEL_REQUEST_DIAGNOSTIC_KIND_MAX_CHARS: usize = 96;
pub const MODEL_REQUEST_DIAGNOSTIC_MESSAGE_MAX_CHARS: usize = 512;
pub const MODEL_REQUEST_DIAGNOSTIC_PARAMETER_MAX_CHARS: usize = 128;

impl ModelRequestDiagnostic {
    pub fn new(
        kind: impl Into<String>,
        message: impl Into<String>,
        parameter: Option<String>,
    ) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
            parameter,
        }
        .sanitized()
    }

    pub fn sanitized(mut self) -> Self {
        self.kind = sanitize_diagnostic_field(&self.kind, MODEL_REQUEST_DIAGNOSTIC_KIND_MAX_CHARS);
        self.message =
            sanitize_diagnostic_field(&self.message, MODEL_REQUEST_DIAGNOSTIC_MESSAGE_MAX_CHARS);
        self.parameter = self
            .parameter
            .take()
            .map(|parameter| {
                sanitize_diagnostic_field(&parameter, MODEL_REQUEST_DIAGNOSTIC_PARAMETER_MAX_CHARS)
            })
            .filter(|parameter| !parameter.is_empty());
        self
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Pending,
    Running,
    WaitingApproval,
    WaitingContinuation,
    RunningTool,
    Compacting,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnState {
    pub protocol_version: u32,
    pub session_id: String,
    pub turn_id: String,
    pub status: TurnStatus,
    pub correlation_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub started_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    #[serde(
        default,
        with = "time::serde::rfc3339::option",
        skip_serializing_if = "Option::is_none"
    )]
    pub ended_at: Option<OffsetDateTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    pub counters: TurnStateCounters,
}

impl TurnState {
    pub fn new(session_id: impl Into<String>, turn_id: impl Into<String>) -> Self {
        let session_id = session_id.into();
        let turn_id = turn_id.into();
        let now = OffsetDateTime::now_utc();
        Self {
            protocol_version: IKAROS_PROTOCOL_VERSION,
            correlation_id: turn_correlation_id(&session_id, &turn_id),
            session_id,
            turn_id,
            status: TurnStatus::Pending,
            started_at: now,
            updated_at: now,
            ended_at: None,
            last_event_id: None,
            last_event_kind: None,
            waiting_on: None,
            stop_reason: None,
            error: None,
            counters: TurnStateCounters::default(),
        }
    }

    pub fn observe(&mut self, event: &StateTraceEntry) {
        self.updated_at = event.at;
        self.last_event_id = Some(event.event_id.clone());
        self.last_event_kind = Some(event.event_kind.clone());
        self.status = event.state_after;
        match event.state_after {
            TurnStatus::Completed | TurnStatus::Failed | TurnStatus::Cancelled => {
                self.ended_at = Some(event.at);
            }
            _ => {}
        }
        if let Some(waiting_on) = event.waiting_on.clone() {
            self.waiting_on = Some(waiting_on);
        }
        if let Some(error) = event.error.clone() {
            self.error = Some(error);
        }
        if let Some(stop_reason) = event.stop_reason.clone() {
            self.stop_reason = Some(stop_reason);
        }
        self.counters.observe(event);
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct TurnStateCounters {
    #[serde(default)]
    pub events: u64,
    #[serde(default)]
    pub model_events: u64,
    #[serde(default)]
    pub tool_events: u64,
    #[serde(default)]
    pub approval_events: u64,
    #[serde(default)]
    pub context_events: u64,
    #[serde(default)]
    pub memory_events: u64,
    #[serde(default)]
    pub coding_events: u64,
    #[serde(default)]
    pub continuation_events: u64,
    #[serde(default)]
    pub error_events: u64,
}

impl TurnStateCounters {
    pub fn observe(&mut self, event: &StateTraceEntry) {
        self.events = self.events.saturating_add(1);
        match event.category.as_str() {
            "model" => self.model_events = self.model_events.saturating_add(1),
            "tool" => self.tool_events = self.tool_events.saturating_add(1),
            "approval" => self.approval_events = self.approval_events.saturating_add(1),
            "context" => self.context_events = self.context_events.saturating_add(1),
            "memory" => self.memory_events = self.memory_events.saturating_add(1),
            "coding" => self.coding_events = self.coding_events.saturating_add(1),
            "continuation" => {
                self.continuation_events = self.continuation_events.saturating_add(1);
            }
            "error" => self.error_events = self.error_events.saturating_add(1),
            _ => {}
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StateTraceEntry {
    pub protocol_version: u32,
    pub session_id: String,
    pub turn_id: String,
    pub event_id: String,
    pub correlation_id: String,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub source: String,
    pub category: String,
    pub event_kind: String,
    pub state_before: TurnStatus,
    pub state_after: TurnStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub waiting_on: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
}

impl StateTraceEntry {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        event_id: impl Into<String>,
        at: OffsetDateTime,
        source: impl Into<String>,
        category: impl Into<String>,
        event_kind: impl Into<String>,
        state_before: TurnStatus,
        state_after: TurnStatus,
        payload: Value,
    ) -> Self {
        let session_id = session_id.into();
        let turn_id = turn_id.into();
        Self {
            protocol_version: IKAROS_PROTOCOL_VERSION,
            correlation_id: turn_correlation_id(&session_id, &turn_id),
            session_id,
            turn_id,
            event_id: event_id.into(),
            at,
            source: source.into(),
            category: category.into(),
            event_kind: event_kind.into(),
            state_before,
            state_after,
            title: None,
            detail: None,
            waiting_on: None,
            stop_reason: None,
            error: None,
            payload,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TurnStateSnapshot {
    pub state: TurnState,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<StateTraceEntry>,
}

impl TurnStateSnapshot {
    pub fn from_trace(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        trace: Vec<StateTraceEntry>,
    ) -> Self {
        let mut state = TurnState::new(session_id, turn_id);
        for entry in &trace {
            state.observe(entry);
        }
        Self { state, trace }
    }
}

pub fn turn_correlation_id(session_id: &str, turn_id: &str) -> String {
    format!("session:{session_id}:turn:{turn_id}")
}

fn sanitize_diagnostic_field(value: &str, max_chars: usize) -> String {
    let redacted = redact_secrets(value);
    let normalized = redacted.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&normalized, max_chars)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    const MARKER: &str = "...[truncated]";
    if value.chars().count() <= max_chars {
        return value.to_owned();
    }
    let marker_len = MARKER.chars().count();
    if max_chars <= marker_len {
        return MARKER.chars().take(max_chars).collect();
    }
    let keep = max_chars - marker_len;
    let mut truncated = value.chars().take(keep).collect::<String>();
    truncated.push_str(MARKER);
    truncated
}

fn redact_secrets(input: &str) -> String {
    let mut output = String::new();
    let mut token = String::new();
    for ch in input.chars() {
        if ch.is_whitespace() {
            push_redacted_token(&mut output, &token);
            token.clear();
            output.push(ch);
        } else {
            token.push(ch);
        }
    }
    push_redacted_token(&mut output, &token);
    output
}

fn push_redacted_token(output: &mut String, token: &str) {
    if token.is_empty() {
        return;
    }
    let is_assignment_secret = token
        .split_once('=')
        .is_some_and(|(key, _)| is_secret_assignment_key(key));
    if token.contains("sk-") {
        output.push_str("[REDACTED_SECRET]");
    } else if is_assignment_secret {
        let key = token.split_once('=').map_or(token, |(key, _)| key);
        let _ = write!(output, "{key}=[REDACTED_SECRET]");
    } else {
        output.push_str(token);
    }
}

fn is_secret_assignment_key(key: &str) -> bool {
    let normalized: String = key
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    matches!(
        normalized.as_str(),
        "apikey" | "accesstoken" | "authtoken" | "token" | "password" | "secret" | "privatekey"
    ) || normalized.ends_with("apikey")
        || normalized.ends_with("accesstoken")
        || normalized.ends_with("authtoken")
        || normalized.ends_with("token")
        || normalized.ends_with("password")
        || normalized.ends_with("secret")
        || normalized.ends_with("privatekey")
}
