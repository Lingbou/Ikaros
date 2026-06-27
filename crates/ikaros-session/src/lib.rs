// SPDX-License-Identifier: GPL-3.0-only
//! Session, turn, and runtime event persistence.

mod sink;
mod sqlite;
mod store;
mod types;

pub use ikaros_protocol::{
    IKAROS_PROTOCOL_NAME, IKAROS_PROTOCOL_VERSION, StateTraceEntry, TurnState, TurnStateSnapshot,
    TurnStatus, WireEnvelope,
};
pub use sink::{
    CollectingAgentEventSink, FanoutAgentEventSink, NoopAgentEventSink, PersistingAgentEventSink,
    PersistingAgentTurnSink, noop_agent_event_sink,
};
pub use sqlite::SqliteSessionStore;
pub use store::{SessionStore, SessionWriter};
pub use types::{
    AgentEvent, AgentEventId, AgentEventKind, AgentEventSink, AgentEventSource, AgentSessionId,
    AgentTurnId, ApprovalRecord, ApprovalStatus, ContinuationId, EventId, SessionBranch,
    SessionBranchSummaryInput, SessionCompactionInput, SessionContinuation,
    SessionContinuationClaim, SessionContinuationInput, SessionContinuationKind,
    SessionContinuationStatus, SessionContinuationStatusReason, SessionEntry, SessionEntryId,
    SessionEntryKind, SessionId, SessionInput, SessionInputAdmission, SessionInputId,
    SessionInputStatus, SessionRecord, SessionReplay, SessionReplayPage, SessionRetryInput,
    SessionSearchHit, SessionSearchIndex, SessionSearchQuery, SessionSource, SessionTimelineItem,
    SessionTimelinePage, SessionTimelineQuery, SessionTimelineRecord, SessionTurnRecord,
    SessionTurnStatus, TurnId, agent_events_to_state_trace,
};
