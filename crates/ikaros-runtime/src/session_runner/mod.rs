// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopInput, AgentLoopReport,
    AgentLoopStopReason, AgentRuntime,
};
use ikaros_core::{IkarosError, Result, redact_secrets};
use ikaros_harness::{CancellationToken, ExecutionSession, SkillRegistry};
use ikaros_models::ModelProvider;
use ikaros_session::{
    CollectingAgentEventSink, ContinuationId, FanoutAgentEventSink, SessionBranchSummaryInput,
    SessionCompactionInput, SessionContinuation, SessionContinuationClaim,
    SessionContinuationInput, SessionContinuationKind, SessionEntry, SessionEntryId,
    SessionRetryInput, SessionStore, TurnId,
};
use std::collections::VecDeque;

mod cancellation;
mod continuations;
mod entries;
mod payload;
mod phase;
mod turn;
mod types;

use cancellation::{ensure_continuation_cancelled, poll_durable_continuation_cancel};
use payload::{
    continuation_payload_str, failed_tool_result_continuation_payload,
    recoverable_tool_result_retry, tool_result_continuation_payload,
};
pub use phase::AgentHarnessPhase;
use phase::AgentHarnessPhaseGuard;
pub use types::{
    AgentHarnessConfig, AgentHarnessContinuation, AgentHarnessMessage, AgentHarnessPendingCounts,
    AgentHarnessTurn,
};

pub struct AgentHarness<'a> {
    config: AgentHarnessConfig,
    runtime: &'a dyn AgentRuntime,
    provider: &'a dyn ModelProvider,
    session: &'a ExecutionSession,
    registry: &'a SkillRegistry,
    event_sink: &'a dyn AgentEventSink,
    continuation_store: Option<&'a dyn SessionStore>,
    phase: AgentHarnessPhase,
    steer_queue: VecDeque<AgentHarnessMessage>,
    follow_up_queue: VecDeque<AgentHarnessMessage>,
    next_turn_queue: VecDeque<AgentHarnessMessage>,
}

impl<'a> AgentHarness<'a> {
    pub fn new(
        config: AgentHarnessConfig,
        runtime: &'a dyn AgentRuntime,
        provider: &'a dyn ModelProvider,
        session: &'a ExecutionSession,
        registry: &'a SkillRegistry,
        event_sink: &'a dyn AgentEventSink,
    ) -> Self {
        Self {
            config,
            runtime,
            provider,
            session,
            registry,
            event_sink,
            continuation_store: None,
            phase: AgentHarnessPhase::Idle,
            steer_queue: VecDeque::new(),
            follow_up_queue: VecDeque::new(),
            next_turn_queue: VecDeque::new(),
        }
    }

    pub fn with_continuation_store(mut self, store: &'a dyn SessionStore) -> Self {
        self.continuation_store = Some(store);
        self
    }

    pub fn phase(&self) -> AgentHarnessPhase {
        self.phase
    }

    pub fn pending_counts(&self) -> AgentHarnessPendingCounts {
        AgentHarnessPendingCounts {
            steer: self.steer_queue.len(),
            follow_up: self.follow_up_queue.len(),
            next_turn: self.next_turn_queue.len(),
        }
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.config.options.cancellation.clone()
    }

    pub fn cancel(&self) {
        self.config.options.cancellation.cancel();
    }

    pub(super) fn run_phase<T>(
        &mut self,
        phase: AgentHarnessPhase,
        operation: impl FnOnce(&AgentHarnessConfig) -> Result<T>,
    ) -> Result<T> {
        let config = self.config.clone();
        let _phase_guard = AgentHarnessPhaseGuard::enter(&mut self.phase, phase)?;
        operation(&config)
    }
}

#[cfg(test)]
mod tests;
