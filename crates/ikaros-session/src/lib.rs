// SPDX-License-Identifier: GPL-3.0-only
//! Session, turn, and runtime event persistence.

mod sink;
mod sqlite;
mod store;
mod types;

pub use sink::{
    NoopAgentEventSink, PersistingAgentEventSink, PersistingAgentTurnSink, noop_agent_event_sink,
};
pub use sqlite::SqliteSessionStore;
pub use store::{SessionStore, SessionWriter};
pub use types::{
    AgentEvent, AgentEventId, AgentEventKind, AgentEventSink, AgentEventSource, AgentSessionId,
    AgentTurnId, ApprovalRecord, ApprovalStatus, ContinuationId, EventId, SessionBranch,
    SessionBranchSummaryInput, SessionCompactionInput, SessionContinuation,
    SessionContinuationClaim, SessionContinuationInput, SessionContinuationKind,
    SessionContinuationStatus, SessionEntry, SessionEntryId, SessionEntryKind, SessionId,
    SessionRecord, SessionReplay, SessionRetryInput, SessionSearchHit, SessionSearchIndex,
    SessionSearchQuery, SessionSource, TurnId,
};
