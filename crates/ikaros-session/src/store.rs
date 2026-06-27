// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    AgentEvent, ApprovalRecord, ContinuationId, SessionBranch, SessionBranchSummaryInput,
    SessionCompactionInput, SessionContinuation, SessionContinuationClaim,
    SessionContinuationInput, SessionEntry, SessionEntryId, SessionId, SessionInput,
    SessionInputAdmission, SessionInputId, SessionRecord, SessionReplay, SessionReplayPage,
    SessionRetryInput, SessionSearchHit, SessionSearchQuery, SessionTimelineItem,
    SessionTimelinePage, SessionTimelineQuery, SessionTurnRecord,
};
use ikaros_core::Result;
use time::OffsetDateTime;

pub trait SessionWriter: Send {
    fn promote_input(&mut self, input_id: &SessionInputId) -> Result<()>;
    fn append_entry(&mut self, entry: &SessionEntry) -> Result<()>;
    fn append_agent_event(&mut self, event: &AgentEvent) -> Result<()>;
    fn append_approval(&mut self, approval: &ApprovalRecord) -> Result<()>;
    fn commit(self: Box<Self>) -> Result<()>;
    fn rollback(self: Box<Self>) -> Result<()>;
}

pub trait SessionStore: Send + Sync {
    fn upsert_session(&self, session: &SessionRecord) -> Result<()>;
    fn finish_session(&self, session_id: &SessionId, ended_at: OffsetDateTime) -> Result<()>;
    fn begin_turn(
        &self,
        session: &SessionRecord,
        turn_id: &crate::TurnId,
    ) -> Result<Box<dyn SessionWriter>>;
    fn append_entry(&self, entry: &SessionEntry) -> Result<()>;
    fn append_agent_event(&self, event: &AgentEvent) -> Result<()>;
    fn append_approval(&self, approval: &ApprovalRecord) -> Result<()>;
    fn get_session(&self, session_id: &SessionId) -> Result<Option<SessionRecord>>;
    fn session_entry(&self, entry_id: &SessionEntryId) -> Result<Option<SessionEntry>>;
    fn session_entries(&self, session_id: &SessionId) -> Result<Vec<SessionEntry>>;
    fn active_branch(&self, session_id: &SessionId) -> Result<Option<SessionBranch>>;
    fn set_active_leaf(&self, session_id: &SessionId, entry_id: &SessionEntryId) -> Result<()>;
    fn append_branch_summary(&self, input: &SessionBranchSummaryInput) -> Result<SessionEntry>;
    fn append_compaction(&self, input: &SessionCompactionInput) -> Result<SessionEntry>;
    fn append_retry_marker(&self, input: &SessionRetryInput) -> Result<SessionEntry>;
    fn admit_input(&self, input: &SessionInputAdmission) -> Result<SessionInput>;
    fn promote_input(
        &self,
        input_id: &SessionInputId,
        turn_id: &crate::TurnId,
    ) -> Result<Option<SessionInput>>;
    fn cancel_input(&self, input_id: &SessionInputId, reason: &str)
    -> Result<Option<SessionInput>>;
    fn session_inputs(&self, session_id: &SessionId) -> Result<Vec<SessionInput>>;
    fn upsert_turn(&self, turn: &SessionTurnRecord) -> Result<()>;
    fn session_turn(
        &self,
        session_id: &SessionId,
        turn_id: &crate::TurnId,
    ) -> Result<Option<SessionTurnRecord>>;
    fn session_turns(&self, session_id: &SessionId) -> Result<Vec<SessionTurnRecord>>;
    fn branch_from_entry(&self, input: &SessionBranchSummaryInput) -> Result<SessionEntry> {
        self.append_branch_summary(input)
    }
    fn retry_from_entry(&self, input: &SessionRetryInput) -> Result<SessionEntry> {
        self.append_retry_marker(input)
    }
    fn search_entries(&self, query: &SessionSearchQuery) -> Result<Vec<SessionSearchHit>>;
    fn enqueue_continuation(&self, input: &SessionContinuationInput)
    -> Result<SessionContinuation>;
    fn claim_next_continuation(
        &self,
        claim: &SessionContinuationClaim,
    ) -> Result<Option<SessionContinuation>>;
    fn complete_continuation(
        &self,
        continuation_id: &ContinuationId,
        payload: serde_json::Value,
    ) -> Result<Option<SessionContinuation>>;
    fn fail_continuation(
        &self,
        continuation_id: &ContinuationId,
        error: &str,
    ) -> Result<Option<SessionContinuation>>;
    fn cancel_continuation(
        &self,
        continuation_id: &ContinuationId,
        reason: &str,
    ) -> Result<Option<SessionContinuation>>;
    fn requeue_continuation(
        &self,
        continuation_id: &ContinuationId,
        reason: &str,
        payload: serde_json::Value,
    ) -> Result<Option<SessionContinuation>>;
    fn continuations(&self, session_id: &SessionId) -> Result<Vec<SessionContinuation>>;
    fn session_timeline(&self, session_id: &SessionId) -> Result<Vec<SessionTimelineItem>>;
    fn session_timeline_page(&self, query: &SessionTimelineQuery) -> Result<SessionTimelinePage>;
    fn agent_events(&self, session_id: &SessionId) -> Result<Vec<AgentEvent>>;
    fn approval_record(&self, approval_id: &str) -> Result<Option<ApprovalRecord>>;
    fn approvals(&self, session_id: &SessionId) -> Result<Vec<ApprovalRecord>>;

    fn replay_session(&self, session_id: &SessionId) -> Result<Option<SessionReplay>> {
        let Some(session) = self.get_session(session_id)? else {
            return Ok(None);
        };
        Ok(Some(SessionReplay {
            entries: self.session_entries(session_id)?,
            agent_events: self.agent_events(session_id)?,
            approvals: self.approvals(session_id)?,
            session,
        }))
    }

    fn replay_session_page(
        &self,
        session_id: &SessionId,
        page: usize,
        page_size: usize,
    ) -> Result<Option<SessionReplayPage>> {
        let Some(session) = self.get_session(session_id)? else {
            return Ok(None);
        };
        let page = page.max(1);
        let page_size = page_size.max(1);
        let start = page.saturating_sub(1).saturating_mul(page_size);
        let entries = self.session_entries(session_id)?;
        let agent_events = self.agent_events(session_id)?;
        let approvals = self.approvals(session_id)?;
        Ok(Some(SessionReplayPage {
            session,
            page,
            page_size,
            total_entries: entries.len(),
            total_agent_events: agent_events.len(),
            total_approvals: approvals.len(),
            entries: entries.into_iter().skip(start).take(page_size).collect(),
            agent_events: agent_events
                .into_iter()
                .skip(start)
                .take(page_size)
                .collect(),
            approvals: approvals.into_iter().skip(start).take(page_size).collect(),
        }))
    }
}
