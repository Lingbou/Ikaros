// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, AgentLoopInput, AgentLoopOptions,
    AgentLoopReport, AgentRuntime,
};
use ikaros_core::{IkarosError, Result};
use ikaros_harness::{CancellationToken, ExecutionSession, SkillRegistry};
use ikaros_models::ModelProvider;
use ikaros_session::{
    ApprovalRecord, SessionBranchSummaryInput, SessionCompactionInput, SessionContinuation,
    SessionContinuationClaim, SessionContinuationInput, SessionContinuationKind, SessionEntry,
    SessionEntryId, SessionId, SessionRetryInput, SessionStore, TurnId,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentHarnessContinuation {
    Turn(AgentHarnessTurn),
    Entry {
        continuation: SessionContinuation,
        entry: SessionEntry,
    },
}

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

    pub fn enqueue_steer(&mut self, message: AgentHarnessMessage) -> Result<()> {
        self.enqueue_continuation(SessionContinuationKind::Steer, message)
    }

    pub fn enqueue_follow_up(&mut self, message: AgentHarnessMessage) -> Result<()> {
        self.enqueue_continuation(SessionContinuationKind::FollowUp, message)
    }

    pub fn enqueue_next_turn(&mut self, message: AgentHarnessMessage) -> Result<()> {
        self.enqueue_continuation(SessionContinuationKind::NextTurn, message)
    }

    pub fn enqueue_resume(&mut self, message: AgentHarnessMessage) -> Result<()> {
        self.enqueue_continuation(SessionContinuationKind::Resume, message)
    }

    pub async fn run_turn(&mut self, user_input: impl Into<String>) -> Result<AgentHarnessTurn> {
        self.run_user_message(user_input.into()).await
    }

    pub async fn run_continue(&mut self) -> Result<AgentHarnessTurn> {
        if let Some(store) = self.continuation_store {
            let Some(continuation) = store.claim_next_continuation(
                &SessionContinuationClaim::for_session(self.config.session_id.clone())
                    .with_kinds([
                        SessionContinuationKind::Steer,
                        SessionContinuationKind::FollowUp,
                        SessionContinuationKind::NextTurn,
                        SessionContinuationKind::Resume,
                    ])
                    .with_lease_owner("agent_harness"),
            )?
            else {
                return Err(IkarosError::Message(
                    "agent harness has no queued continuation".into(),
                ));
            };
            return self
                .run_durable_message_continuation(store, continuation)
                .await;
        }
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

    pub async fn run_next_continuation(&mut self) -> Result<AgentHarnessContinuation> {
        let Some(store) = self.continuation_store else {
            return self
                .run_continue()
                .await
                .map(AgentHarnessContinuation::Turn);
        };
        let Some(continuation) = store.claim_next_continuation(
            &SessionContinuationClaim::for_session(self.config.session_id.clone())
                .with_kinds([
                    SessionContinuationKind::Steer,
                    SessionContinuationKind::FollowUp,
                    SessionContinuationKind::NextTurn,
                    SessionContinuationKind::Resume,
                    SessionContinuationKind::Compact,
                    SessionContinuationKind::Retry,
                ])
                .with_lease_owner("agent_harness"),
        )?
        else {
            return Err(IkarosError::Message(
                "agent harness has no queued continuation".into(),
            ));
        };
        match continuation.kind {
            SessionContinuationKind::Steer
            | SessionContinuationKind::FollowUp
            | SessionContinuationKind::NextTurn
            | SessionContinuationKind::Resume => self
                .run_durable_message_continuation(store, continuation)
                .await
                .map(AgentHarnessContinuation::Turn),
            SessionContinuationKind::Compact | SessionContinuationKind::Retry => self
                .run_durable_entry_continuation(store, continuation)
                .await
                .map(|(continuation, entry)| AgentHarnessContinuation::Entry {
                    continuation,
                    entry,
                }),
        }
    }

    pub fn enqueue_compaction(
        &mut self,
        parent_entry_id: SessionEntryId,
        summary: impl Into<String>,
        compacted_entry_ids: Vec<SessionEntryId>,
        payload: serde_json::Value,
    ) -> Result<SessionContinuation> {
        let store = self.continuation_store.ok_or_else(|| {
            IkarosError::Message("durable compaction requires a continuation store".into())
        })?;
        let mut input = SessionContinuationInput::new(
            self.config.session_id.clone(),
            SessionContinuationKind::Compact,
        );
        input.priority = SessionContinuationKind::Compact.default_priority();
        input.payload = serde_json::json!({
            "parent_entry_id": parent_entry_id.as_str(),
            "summary": summary.into(),
            "compacted_entry_ids": compacted_entry_ids
                .iter()
                .map(SessionEntryId::as_str)
                .collect::<Vec<_>>(),
            "data": payload,
        });
        store.enqueue_continuation(&input)
    }

    pub fn enqueue_retry_marker(
        &mut self,
        parent_entry_id: SessionEntryId,
        reason: Option<String>,
        payload: serde_json::Value,
    ) -> Result<SessionContinuation> {
        let store = self.continuation_store.ok_or_else(|| {
            IkarosError::Message("durable retry requires a continuation store".into())
        })?;
        let mut input = SessionContinuationInput::new(
            self.config.session_id.clone(),
            SessionContinuationKind::Retry,
        );
        input.priority = SessionContinuationKind::Retry.default_priority();
        input.payload = serde_json::json!({
            "parent_entry_id": parent_entry_id.as_str(),
            "reason": reason,
            "data": payload,
        });
        store.enqueue_continuation(&input)
    }

    fn enqueue_continuation(
        &mut self,
        kind: SessionContinuationKind,
        message: AgentHarnessMessage,
    ) -> Result<()> {
        if let Some(store) = self.continuation_store {
            let mut input = SessionContinuationInput::new(self.config.session_id.clone(), kind);
            input.payload = serde_json::json!({
                "content": message.content,
                "source": "agent_harness",
            });
            store.enqueue_continuation(&input)?;
            return Ok(());
        }
        match kind {
            SessionContinuationKind::Steer => self.steer_queue.push_back(message),
            SessionContinuationKind::FollowUp => self.follow_up_queue.push_back(message),
            SessionContinuationKind::NextTurn => self.next_turn_queue.push_back(message),
            SessionContinuationKind::Resume
            | SessionContinuationKind::Retry
            | SessionContinuationKind::Compact => {
                return Err(IkarosError::Message(format!(
                    "unsupported in-memory continuation kind: {kind:?}"
                )));
            }
        }
        Ok(())
    }

    async fn run_durable_message_continuation(
        &mut self,
        store: &dyn SessionStore,
        continuation: SessionContinuation,
    ) -> Result<AgentHarnessTurn> {
        let content = continuation
            .payload
            .get("content")
            .and_then(|value| value.as_str())
            .ok_or_else(|| {
                IkarosError::Message(format!(
                    "continuation {} has no user content",
                    continuation.continuation_id
                ))
            })?
            .to_owned();
        let continuation_id = continuation.continuation_id.clone();
        let turn_id = continuation.turn_id.clone().unwrap_or_default();
        self.config.turn_id = Some(turn_id.clone());
        self.emit_continuation_event(
            &turn_id,
            AgentEventKind::ContinuationStarted,
            &continuation,
            serde_json::json!({"kind": format!("{:?}", continuation.kind)}),
        )?;
        match self.run_user_message(content).await {
            Ok(turn) => {
                store.complete_continuation(
                    &continuation_id,
                    serde_json::json!({
                        "completed_turn_id": turn.turn_id.as_str(),
                        "stop_reason": format!("{:?}", turn.report.stop_reason),
                    }),
                )?;
                self.emit_continuation_event(
                    &turn.turn_id,
                    AgentEventKind::ContinuationCompleted,
                    &continuation,
                    serde_json::json!({
                        "completed_turn_id": turn.turn_id.as_str(),
                        "stop_reason": format!("{:?}", turn.report.stop_reason),
                    }),
                )?;
                Ok(turn)
            }
            Err(error) => {
                let _ = store.fail_continuation(&continuation_id, &error.to_string());
                let _ = self.emit_continuation_event(
                    &turn_id,
                    AgentEventKind::ContinuationFailed,
                    &continuation,
                    serde_json::json!({"error": error.to_string()}),
                );
                Err(error)
            }
        }
    }

    async fn run_durable_entry_continuation(
        &mut self,
        store: &dyn SessionStore,
        continuation: SessionContinuation,
    ) -> Result<(SessionContinuation, SessionEntry)> {
        let continuation_id = continuation.continuation_id.clone();
        let turn_id = continuation.turn_id.clone().unwrap_or_default();
        self.emit_continuation_event(
            &turn_id,
            AgentEventKind::ContinuationStarted,
            &continuation,
            serde_json::json!({"kind": format!("{:?}", continuation.kind)}),
        )?;
        let result = match continuation.kind {
            SessionContinuationKind::Compact => {
                self.run_compaction_continuation(store, &continuation)
            }
            SessionContinuationKind::Retry => self.run_retry_continuation(store, &continuation),
            other => Err(IkarosError::Message(format!(
                "continuation {continuation_id} is not an entry continuation: {other:?}"
            ))),
        };
        match result {
            Ok(entry) => {
                let continuation = store
                    .complete_continuation(
                        &continuation_id,
                        serde_json::json!({"entry_id": entry.entry_id.as_str()}),
                    )?
                    .ok_or_else(|| {
                        IkarosError::Message(format!(
                            "continuation disappeared while completing: {continuation_id}"
                        ))
                    })?;
                self.emit_continuation_event(
                    &turn_id,
                    AgentEventKind::ContinuationCompleted,
                    &continuation,
                    serde_json::json!({"entry_id": entry.entry_id.as_str()}),
                )?;
                Ok((continuation, entry))
            }
            Err(error) => {
                let _ = store.fail_continuation(&continuation_id, &error.to_string());
                let _ = self.emit_continuation_event(
                    &turn_id,
                    AgentEventKind::ContinuationFailed,
                    &continuation,
                    serde_json::json!({"error": error.to_string()}),
                );
                Err(error)
            }
        }
    }

    fn run_compaction_continuation(
        &mut self,
        store: &dyn SessionStore,
        continuation: &SessionContinuation,
    ) -> Result<SessionEntry> {
        let parent_entry_id = continuation_payload_str(continuation, "parent_entry_id")?;
        let summary = continuation_payload_str(continuation, "summary")?;
        let compacted_entry_ids = continuation
            .payload
            .get("compacted_entry_ids")
            .and_then(|value| value.as_array())
            .ok_or_else(|| {
                IkarosError::Message(format!(
                    "continuation {} missing compacted_entry_ids",
                    continuation.continuation_id
                ))
            })?
            .iter()
            .map(|value| {
                value.as_str().map(SessionEntryId::from).ok_or_else(|| {
                    IkarosError::Message(format!(
                        "continuation {} has non-string compacted entry id",
                        continuation.continuation_id
                    ))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        self.append_compaction(
            store,
            SessionEntryId::from(parent_entry_id),
            summary,
            compacted_entry_ids,
            continuation
                .payload
                .get("data")
                .cloned()
                .unwrap_or_default(),
        )
    }

    fn run_retry_continuation(
        &mut self,
        store: &dyn SessionStore,
        continuation: &SessionContinuation,
    ) -> Result<SessionEntry> {
        let parent_entry_id = continuation_payload_str(continuation, "parent_entry_id")?;
        let reason = continuation
            .payload
            .get("reason")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned);
        self.append_retry_marker(
            store,
            SessionEntryId::from(parent_entry_id),
            reason,
            continuation
                .payload
                .get("data")
                .cloned()
                .unwrap_or_default(),
        )
    }

    fn emit_continuation_event(
        &self,
        turn_id: &TurnId,
        kind: AgentEventKind,
        continuation: &SessionContinuation,
        payload: serde_json::Value,
    ) -> Result<()> {
        self.event_sink.emit(&AgentEvent::new(
            self.config.session_id.clone(),
            turn_id.clone(),
            None,
            AgentEventSource::Harness,
            kind,
            serde_json::json!({
                "continuation_id": continuation.continuation_id.as_str(),
                "continuation_kind": format!("{:?}", continuation.kind),
                "payload": payload,
            }),
        ))
    }

    pub fn append_branch_summary(
        &mut self,
        store: &dyn SessionStore,
        parent_entry_id: SessionEntryId,
        summary: impl Into<String>,
        payload: serde_json::Value,
    ) -> Result<SessionEntry> {
        let summary = summary.into();
        self.run_phase(AgentHarnessPhase::BranchSummary, |harness| {
            store.branch_from_entry(&SessionBranchSummaryInput {
                session_id: harness.config.session_id.clone(),
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
        self.run_phase(AgentHarnessPhase::Compaction, |harness| {
            store.append_compaction(&SessionCompactionInput {
                session_id: harness.config.session_id.clone(),
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
        self.run_phase(AgentHarnessPhase::Retry, |harness| {
            store.retry_from_entry(&SessionRetryInput {
                session_id: harness.config.session_id.clone(),
                parent_entry_id,
                reason,
                payload,
            })
        })
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
        let turn_id = self.config.turn_id.take().unwrap_or_default();
        let input = AgentLoopInput {
            session_id: Some(session_id.as_str().to_owned()),
            turn_id: Some(turn_id.as_str().to_owned()),
            task_id: self.config.task_id.clone(),
            system_prompt: self.config.system_prompt.clone(),
            user_input,
        };
        let emitted_events = Arc::new(Mutex::new(Vec::new()));
        let event_sink = CollectingAgentEventSink {
            downstream: self.event_sink,
            events: emitted_events.clone(),
        };
        let result = self
            .runtime
            .run_turn_with_events(
                input,
                self.provider,
                self.session,
                self.registry,
                &event_sink,
                self.config.options.clone(),
            )
            .await;
        self.phase = AgentHarnessPhase::Idle;
        let mut report = result?;
        let events = collected_events(&emitted_events)?;
        report.events = events.clone();
        Ok(AgentHarnessTurn {
            session_id,
            turn_id,
            events,
            report,
        })
    }

    fn run_phase<T>(
        &mut self,
        phase: AgentHarnessPhase,
        operation: impl FnOnce(&Self) -> Result<T>,
    ) -> Result<T> {
        if self.phase != AgentHarnessPhase::Idle {
            return Err(IkarosError::Message(format!(
                "agent harness is busy in {:?} phase",
                self.phase
            )));
        }
        self.phase = phase;
        let result = operation(self);
        self.phase = AgentHarnessPhase::Idle;
        result
    }
}

struct CollectingAgentEventSink<'a> {
    downstream: &'a dyn AgentEventSink,
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

impl AgentEventSink for CollectingAgentEventSink<'_> {
    fn emit(&self, event: &AgentEvent) -> Result<()> {
        self.downstream.emit(event)?;
        self.events
            .lock()
            .map_err(|_| IkarosError::Message("agent harness event lock poisoned".into()))?
            .push(event.clone());
        Ok(())
    }

    fn emit_approval(&self, approval: &ApprovalRecord) -> Result<()> {
        self.downstream.emit_approval(approval)
    }
}

fn collected_events(events: &Arc<Mutex<Vec<AgentEvent>>>) -> Result<Vec<AgentEvent>> {
    events
        .lock()
        .map(|events| events.clone())
        .map_err(|_| IkarosError::Message("agent harness event lock poisoned".into()))
}

fn continuation_payload_str(continuation: &SessionContinuation, key: &str) -> Result<String> {
    continuation
        .payload
        .get(key)
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            IkarosError::Message(format!(
                "continuation {} missing string payload field {key}",
                continuation.continuation_id
            ))
        })
}

#[cfg(test)]
mod tests;
