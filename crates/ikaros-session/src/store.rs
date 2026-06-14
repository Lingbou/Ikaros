// SPDX-License-Identifier: GPL-3.0-only

use crate::{AgentEvent, ApprovalRecord, SessionEntry, SessionId, SessionRecord, SessionReplay};
use ikaros_core::Result;
use time::OffsetDateTime;

pub trait SessionStore: Send + Sync {
    fn upsert_session(&self, session: &SessionRecord) -> Result<()>;
    fn finish_session(&self, session_id: &SessionId, ended_at: OffsetDateTime) -> Result<()>;
    fn append_entry(&self, entry: &SessionEntry) -> Result<()>;
    fn append_agent_event(&self, event: &AgentEvent) -> Result<()>;
    fn append_approval(&self, approval: &ApprovalRecord) -> Result<()>;
    fn get_session(&self, session_id: &SessionId) -> Result<Option<SessionRecord>>;
    fn session_entries(&self, session_id: &SessionId) -> Result<Vec<SessionEntry>>;
    fn agent_events(&self, session_id: &SessionId) -> Result<Vec<AgentEvent>>;
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
}
