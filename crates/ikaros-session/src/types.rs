// SPDX-License-Identifier: GPL-3.0-only

mod approval;
mod branch;
mod continuation;
mod entry;
mod event;
mod ids;
mod input;
mod replay;
mod search;
mod session;
mod timeline;
mod turn;

pub use approval::{ApprovalRecord, ApprovalStatus};
pub use branch::{
    SessionBranch, SessionBranchSummaryInput, SessionCompactionInput, SessionRetryInput,
};
pub use continuation::{
    SessionContinuation, SessionContinuationClaim, SessionContinuationInput,
    SessionContinuationKind, SessionContinuationStatus, SessionContinuationStatusReason,
};
pub use entry::{SessionEntry, SessionEntryKind};
pub use event::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, agent_events_to_state_trace,
};
pub use ids::{
    AgentEventId, AgentSessionId, AgentTurnId, ContinuationId, EventId, SessionEntryId, SessionId,
    SessionInputId, TurnId,
};
pub use input::{SessionInput, SessionInputAdmission, SessionInputStatus};
pub use replay::{SessionReplay, SessionReplayPage};
pub use search::{SessionSearchHit, SessionSearchIndex, SessionSearchQuery};
pub use session::{SessionRecord, SessionSource};
pub use timeline::{
    SessionTimelineItem, SessionTimelinePage, SessionTimelineQuery, SessionTimelineRecord,
};
pub use turn::{SessionTurnRecord, SessionTurnStatus};

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_protocol::ModelRequestDiagnostic;
    use serde_json::json;

    #[test]
    fn agent_event_kind_model_diagnostic_roundtrip() {
        let diagnostic = ModelRequestDiagnostic {
            kind: "provider_retry_failed".into(),
            message: "provider openai-compatible/gpt-4o retry attempt 1 failed".into(),
            parameter: Some("temperature".into()),
        };
        let kind = AgentEventKind::ModelDiagnostic(diagnostic.clone());
        let value = serde_json::to_value(&kind).expect("serialize");
        assert_eq!(
            value,
            json!({
                "type": "model_diagnostic",
                "data": {
                    "kind": "provider_retry_failed",
                    "message": "provider openai-compatible/gpt-4o retry attempt 1 failed",
                    "parameter": "temperature",
                }
            })
        );
        let restored: AgentEventKind = serde_json::from_value(value).expect("deserialize");
        assert_eq!(restored, kind);
    }
}
