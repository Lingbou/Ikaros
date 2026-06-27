// SPDX-License-Identifier: GPL-3.0-only

use super::{AgentEventId, AgentSessionId, AgentTurnId, ApprovalRecord, EventId};
use ikaros_core::Result;
use ikaros_protocol::{
    ModelRequestDiagnostic, ModelStreamEvent, StateTraceEntry, TurnState, TurnStatus,
};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

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
            event_id: EventId::new(),
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            parent_event_id,
            at: OffsetDateTime::now_utc(),
            source,
            kind,
            payload,
        }
    }

    pub fn correlation_id(&self) -> String {
        ikaros_protocol::turn_correlation_id(self.session_id.as_str(), self.turn_id.as_str())
    }

    pub fn trace_category(&self) -> &'static str {
        self.kind.trace_category()
    }

    pub fn trace_event_kind(&self) -> &'static str {
        self.kind.trace_event_kind()
    }

    pub fn trace_state_after(&self) -> TurnStatus {
        self.kind.trace_state_after()
    }

    pub fn to_state_trace_entry(&self, state_before: TurnStatus) -> StateTraceEntry {
        let state_after = self.trace_state_after();
        let mut entry = StateTraceEntry::new(
            self.session_id.as_str(),
            self.turn_id.as_str(),
            self.event_id.as_str(),
            self.at,
            self.source.as_str(),
            self.trace_category(),
            self.trace_event_kind(),
            state_before,
            state_after,
            self.payload.clone(),
        );
        entry.title = Some(self.trace_event_kind().to_owned());
        entry.detail = self.kind.trace_detail();
        entry.waiting_on = self.kind.trace_waiting_on();
        entry.error = self.kind.trace_error_message(&self.payload);
        entry.stop_reason = self.kind.trace_stop_reason(&self.payload);
        entry
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
    Memory,
    Audit,
}

impl AgentEventSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Runtime => "runtime",
            Self::User => "user",
            Self::Model => "model",
            Self::Tool => "tool",
            Self::Harness => "harness",
            Self::Context => "context",
            Self::Memory => "memory",
            Self::Audit => "audit",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentEventKind {
    SessionStart,
    TurnStart,
    UserMessage,
    ModelStream(ModelStreamEvent),
    ModelDiagnostic(ModelRequestDiagnostic),
    ToolCallStarted,
    ToolCallOutputDelta,
    ToolCallCompleted,
    ToolCallFailed,
    ToolCallCancelled,
    ContextDiff,
    ContextCompacted,
    MemoryLifecycle,
    CodingTurn,
    AuditAnchor,
    ContinuationStarted,
    ContinuationCompleted,
    ContinuationFailed,
    ContinuationCancelled,
    ApprovalRequested,
    ApprovalResolved,
    TurnEnd,
    Error,
}

impl AgentEventKind {
    pub fn trace_category(&self) -> &'static str {
        match self {
            Self::ModelStream(_) | Self::ModelDiagnostic(_) => "model",
            Self::ToolCallStarted
            | Self::ToolCallOutputDelta
            | Self::ToolCallCompleted
            | Self::ToolCallFailed
            | Self::ToolCallCancelled => "tool",
            Self::ApprovalRequested | Self::ApprovalResolved => "approval",
            Self::ContextDiff | Self::ContextCompacted => "context",
            Self::MemoryLifecycle => "memory",
            Self::CodingTurn => "coding",
            Self::ContinuationStarted
            | Self::ContinuationCompleted
            | Self::ContinuationFailed
            | Self::ContinuationCancelled => "continuation",
            Self::Error => "error",
            Self::AuditAnchor => "audit",
            Self::SessionStart | Self::TurnStart | Self::TurnEnd | Self::UserMessage => "session",
        }
    }

    pub fn trace_event_kind(&self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::TurnStart => "turn_start",
            Self::UserMessage => "user_message",
            Self::ModelStream(stream_event) => match stream_event {
                ModelStreamEvent::Start { .. } => "model_stream_start",
                ModelStreamEvent::TextDelta(_) => "model_text_delta",
                ModelStreamEvent::ReasoningDelta(_) => "model_reasoning_delta",
                ModelStreamEvent::ToolCallStart { .. } => "model_tool_call_start",
                ModelStreamEvent::ToolCallDelta { .. } => "model_tool_call_delta",
                ModelStreamEvent::ToolCallEnd { .. } => "model_tool_call_end",
                ModelStreamEvent::RefusalDelta(_) => "model_refusal_delta",
                ModelStreamEvent::Usage(_) => "model_usage",
                ModelStreamEvent::Error { .. } => "model_stream_error",
                ModelStreamEvent::Done => "model_stream_done",
            },
            Self::ModelDiagnostic(_) => "model_diagnostic",
            Self::ToolCallStarted => "tool_call_started",
            Self::ToolCallOutputDelta => "tool_call_output_delta",
            Self::ToolCallCompleted => "tool_call_completed",
            Self::ToolCallFailed => "tool_call_failed",
            Self::ToolCallCancelled => "tool_call_cancelled",
            Self::ContextDiff => "context_diff",
            Self::ContextCompacted => "context_compacted",
            Self::MemoryLifecycle => "memory_lifecycle",
            Self::CodingTurn => "coding_turn",
            Self::AuditAnchor => "audit_anchor",
            Self::ContinuationStarted => "continuation_started",
            Self::ContinuationCompleted => "continuation_completed",
            Self::ContinuationFailed => "continuation_failed",
            Self::ContinuationCancelled => "continuation_cancelled",
            Self::ApprovalRequested => "approval_requested",
            Self::ApprovalResolved => "approval_resolved",
            Self::TurnEnd => "turn_end",
            Self::Error => "error",
        }
    }

    pub fn trace_state_after(&self) -> TurnStatus {
        match self {
            Self::SessionStart => TurnStatus::Pending,
            Self::TurnStart
            | Self::UserMessage
            | Self::ModelStream(ModelStreamEvent::Start { .. })
            | Self::ModelStream(ModelStreamEvent::TextDelta(_))
            | Self::ModelStream(ModelStreamEvent::ReasoningDelta(_))
            | Self::ModelStream(ModelStreamEvent::RefusalDelta(_))
            | Self::ModelStream(ModelStreamEvent::Usage(_))
            | Self::ModelStream(ModelStreamEvent::Done)
            | Self::ModelDiagnostic(_)
            | Self::ContextDiff
            | Self::MemoryLifecycle
            | Self::CodingTurn
            | Self::AuditAnchor
            | Self::ContinuationCompleted
            | Self::ApprovalResolved
            | Self::ToolCallCompleted => TurnStatus::Running,
            Self::ModelStream(ModelStreamEvent::ToolCallStart { .. })
            | Self::ModelStream(ModelStreamEvent::ToolCallDelta { .. })
            | Self::ModelStream(ModelStreamEvent::ToolCallEnd { .. })
            | Self::ToolCallStarted
            | Self::ToolCallOutputDelta => TurnStatus::RunningTool,
            Self::ApprovalRequested => TurnStatus::WaitingApproval,
            Self::ContinuationStarted => TurnStatus::WaitingContinuation,
            Self::ContextCompacted => TurnStatus::Compacting,
            Self::TurnEnd => TurnStatus::Completed,
            Self::ToolCallFailed
            | Self::ContinuationFailed
            | Self::ModelStream(ModelStreamEvent::Error { .. })
            | Self::Error => TurnStatus::Failed,
            Self::ToolCallCancelled | Self::ContinuationCancelled => TurnStatus::Cancelled,
        }
    }

    pub fn trace_detail(&self) -> Option<String> {
        match self {
            Self::ModelStream(ModelStreamEvent::Start { provider, model }) => {
                Some(format!("provider={provider} model={model}"))
            }
            Self::ModelStream(ModelStreamEvent::ToolCallStart { id, name }) => {
                Some(format!("tool_call_id={id} name={name}"))
            }
            Self::ModelStream(ModelStreamEvent::ToolCallDelta { id, .. }) => {
                Some(format!("tool_call_id={id}"))
            }
            Self::ModelStream(ModelStreamEvent::ToolCallEnd { id }) => {
                Some(format!("tool_call_id={id}"))
            }
            Self::ModelStream(ModelStreamEvent::Usage(usage)) => Some(format!(
                "total_tokens={}",
                usage.total_or_prompt_completion()
            )),
            Self::ModelStream(ModelStreamEvent::Error { message }) => Some(message.clone()),
            Self::ModelDiagnostic(diagnostic) => {
                Some(format!("{}: {}", diagnostic.kind, diagnostic.message))
            }
            _ => None,
        }
    }

    pub fn trace_waiting_on(&self) -> Option<String> {
        match self {
            Self::ApprovalRequested => Some("approval".into()),
            Self::ContinuationStarted => Some("continuation".into()),
            _ => None,
        }
    }

    pub fn trace_error_message(&self, payload: &serde_json::Value) -> Option<String> {
        match self {
            Self::ModelStream(ModelStreamEvent::Error { message }) => Some(message.clone()),
            Self::ToolCallFailed | Self::ContinuationFailed | Self::Error => payload
                .get("error")
                .or_else(|| payload.get("message"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            _ => None,
        }
    }

    pub fn trace_stop_reason(&self, payload: &serde_json::Value) -> Option<String> {
        match self {
            Self::TurnEnd => payload
                .get("stop_reason")
                .or_else(|| payload.get("reason"))
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
            _ => None,
        }
    }
}

pub trait AgentEventSink: Send + Sync {
    fn emit(&self, event: &AgentEvent) -> Result<()>;

    fn emit_approval(&self, _approval: &ApprovalRecord) -> Result<()> {
        Ok(())
    }
}

pub fn agent_events_to_state_trace(
    events: &[AgentEvent],
    turn_id: Option<&str>,
) -> Vec<StateTraceEntry> {
    let mut state = turn_id.and_then(|turn_id| {
        events
            .iter()
            .find(|event| event.turn_id.as_str() == turn_id)
            .map(|event| TurnState::new(event.session_id.as_str(), turn_id))
    });
    let mut trace = Vec::new();
    for event in events {
        if turn_id.is_some_and(|turn_id| event.turn_id.as_str() != turn_id) {
            continue;
        }
        let mut current = state
            .take()
            .unwrap_or_else(|| TurnState::new(event.session_id.as_str(), event.turn_id.as_str()));
        let entry = event.to_state_trace_entry(current.status);
        current.observe(&entry);
        state = Some(current);
        trace.push(entry);
    }
    trace
}
