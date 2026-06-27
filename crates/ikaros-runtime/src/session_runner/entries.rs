// SPDX-License-Identifier: GPL-3.0-only

use super::*;

impl<'a> AgentHarness<'a> {
    pub fn append_branch_summary(
        &mut self,
        store: &dyn SessionStore,
        parent_entry_id: SessionEntryId,
        summary: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<SessionEntry> {
        let summary = summary.into();
        self.run_phase(AgentHarnessPhase::BranchSummary, |config| {
            store.branch_from_entry(&SessionBranchSummaryInput {
                session_id: config.session_id.clone(),
                parent_entry_id,
                summary,
                payload,
            })
        })
    }

    pub fn append_compaction(
        &mut self,
        store: &dyn SessionStore,
        parent_entry_id: SessionEntryId,
        summary: impl Into<String>,
        compacted_entry_ids: Vec<SessionEntryId>,
        payload: serde_json::Value,
    ) -> Result<SessionEntry> {
        let summary = summary.into();
        self.run_phase(AgentHarnessPhase::Compaction, |config| {
            store.append_compaction(&SessionCompactionInput {
                session_id: config.session_id.clone(),
                parent_entry_id,
                summary,
                compacted_entry_ids,
                payload,
            })
        })
    }

    pub fn append_retry_marker(
        &mut self,
        store: &dyn SessionStore,
        parent_entry_id: SessionEntryId,
        reason: Option<String>,
        payload: serde_json::Value,
    ) -> Result<SessionEntry> {
        self.run_phase(AgentHarnessPhase::Retry, |config| {
            store.retry_from_entry(&SessionRetryInput {
                session_id: config.session_id.clone(),
                parent_entry_id,
                reason,
                payload,
            })
        })
    }
}
