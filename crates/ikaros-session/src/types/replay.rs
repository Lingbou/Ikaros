// SPDX-License-Identifier: GPL-3.0-only

use super::{AgentEvent, ApprovalRecord, SessionEntry, SessionRecord, agent_events_to_state_trace};
use ikaros_protocol::{StateTraceEntry, TurnStateSnapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionReplay {
    pub session: SessionRecord,
    pub entries: Vec<SessionEntry>,
    pub agent_events: Vec<AgentEvent>,
    pub approvals: Vec<ApprovalRecord>,
}

impl SessionReplay {
    pub fn turn_state_snapshot(&self, turn_id: impl AsRef<str>) -> TurnStateSnapshot {
        let turn_id = turn_id.as_ref();
        let trace = agent_events_to_state_trace(&self.agent_events, Some(turn_id));
        TurnStateSnapshot::from_trace(self.session.session_id.as_str(), turn_id, trace)
    }

    pub fn turn_state_snapshots(&self) -> Vec<TurnStateSnapshot> {
        let mut turn_ids = self
            .agent_events
            .iter()
            .map(|event| event.turn_id.as_str().to_owned())
            .collect::<Vec<_>>();
        turn_ids.sort();
        turn_ids.dedup();
        turn_ids
            .into_iter()
            .map(|turn_id| self.turn_state_snapshot(turn_id))
            .collect()
    }

    pub fn state_trace(&self) -> Vec<StateTraceEntry> {
        agent_events_to_state_trace(&self.agent_events, None)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionReplayPage {
    pub session: SessionRecord,
    pub page: usize,
    pub page_size: usize,
    pub total_entries: usize,
    pub total_agent_events: usize,
    pub total_approvals: usize,
    pub entries: Vec<SessionEntry>,
    pub agent_events: Vec<AgentEvent>,
    pub approvals: Vec<ApprovalRecord>,
}

impl SessionReplayPage {
    pub fn state_trace(&self) -> Vec<StateTraceEntry> {
        agent_events_to_state_trace(&self.agent_events, None)
    }
}
