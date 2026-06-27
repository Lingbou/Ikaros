// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{IkarosError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentHarnessPhase {
    Idle,
    Turn,
    Compaction,
    BranchSummary,
    Retry,
}

pub(super) struct AgentHarnessPhaseGuard<'a> {
    phase: &'a mut AgentHarnessPhase,
}

impl<'a> AgentHarnessPhaseGuard<'a> {
    pub(super) fn enter(phase: &'a mut AgentHarnessPhase, next: AgentHarnessPhase) -> Result<Self> {
        if *phase != AgentHarnessPhase::Idle {
            return Err(IkarosError::Message(format!(
                "agent harness is busy in {:?} phase",
                phase
            )));
        }
        *phase = next;
        Ok(Self { phase })
    }
}

impl Drop for AgentHarnessPhaseGuard<'_> {
    fn drop(&mut self) {
        *self.phase = AgentHarnessPhase::Idle;
    }
}
