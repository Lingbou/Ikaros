// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::{
    AgentEvent, AgentEventSink, AgentLoopInput, AgentLoopOptions, AgentLoopReport, AgentRuntime,
};
use ikaros_core::{IkarosError, Result};
use ikaros_harness::{ExecutionSession, SkillRegistry};
use ikaros_models::ModelProvider;
use ikaros_session::{SessionId, TurnId};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentHarnessPhase {
    Idle,
    Turn,
    Compaction,
    BranchSummary,
    Retry,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentHarnessConfig {
    pub session_id: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<TurnId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub system_prompt: String,
    pub options: AgentLoopOptions,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentHarnessMessage {
    pub content: String,
}

impl AgentHarnessMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentHarnessPendingCounts {
    pub steer: usize,
    pub follow_up: usize,
    pub next_turn: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentHarnessTurn {
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub events: Vec<AgentEvent>,
    pub report: AgentLoopReport,
}

pub struct AgentHarness<'a> {
    config: AgentHarnessConfig,
    runtime: &'a dyn AgentRuntime,
    provider: &'a dyn ModelProvider,
    session: &'a ExecutionSession,
    registry: &'a SkillRegistry,
    event_sink: &'a dyn AgentEventSink,
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
            phase: AgentHarnessPhase::Idle,
            steer_queue: VecDeque::new(),
            follow_up_queue: VecDeque::new(),
            next_turn_queue: VecDeque::new(),
        }
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

    pub fn enqueue_steer(&mut self, message: AgentHarnessMessage) {
        self.steer_queue.push_back(message);
    }

    pub fn enqueue_follow_up(&mut self, message: AgentHarnessMessage) {
        self.follow_up_queue.push_back(message);
    }

    pub fn enqueue_next_turn(&mut self, message: AgentHarnessMessage) {
        self.next_turn_queue.push_back(message);
    }

    pub async fn run_turn(&mut self, user_input: impl Into<String>) -> Result<AgentHarnessTurn> {
        self.run_user_message(user_input.into()).await
    }

    pub async fn run_continue(&mut self) -> Result<AgentHarnessTurn> {
        let message = self
            .steer_queue
            .pop_front()
            .or_else(|| self.follow_up_queue.pop_front())
            .or_else(|| self.next_turn_queue.pop_front())
            .ok_or_else(|| {
                IkarosError::Message("agent harness has no queued continuation".into())
            })?;
        self.run_user_message(message.content).await
    }

    async fn run_user_message(&mut self, user_input: String) -> Result<AgentHarnessTurn> {
        if self.phase != AgentHarnessPhase::Idle {
            return Err(IkarosError::Message(format!(
                "agent harness is busy in {:?} phase",
                self.phase
            )));
        }
        self.phase = AgentHarnessPhase::Turn;
        let session_id = self.config.session_id.clone();
        let turn_id = self.config.turn_id.clone().unwrap_or_default();
        let input = AgentLoopInput {
            session_id: Some(session_id.as_str().to_owned()),
            turn_id: Some(turn_id.as_str().to_owned()),
            task_id: self.config.task_id.clone(),
            system_prompt: self.config.system_prompt.clone(),
            user_input,
        };
        let result = self
            .runtime
            .run_turn_with_events(
                input,
                self.provider,
                self.session,
                self.registry,
                self.event_sink,
                self.config.options.clone(),
            )
            .await;
        self.phase = AgentHarnessPhase::Idle;
        let report = result?;
        Ok(AgentHarnessTurn {
            session_id,
            turn_id,
            events: report.events.clone(),
            report,
        })
    }
}

#[cfg(test)]
mod tests;
