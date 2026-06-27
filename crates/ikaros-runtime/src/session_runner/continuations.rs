// SPDX-License-Identifier: GPL-3.0-only

use super::*;

impl<'a> AgentHarness<'a> {
    pub fn cancel_continuation(
        &self,
        continuation_id: &ContinuationId,
        reason: &str,
    ) -> Result<Option<SessionContinuation>> {
        let store = self.continuation_store.ok_or_else(|| {
            IkarosError::Message("durable continuation cancellation requires a store".into())
        })?;
        let Some(cancelled) = store.cancel_continuation(continuation_id, reason)? else {
            return Ok(None);
        };
        let turn_id = cancelled
            .turn_id
            .clone()
            .or_else(|| self.config.turn_id.clone())
            .unwrap_or_default();
        self.emit_continuation_event(
            &turn_id,
            AgentEventKind::ContinuationCancelled,
            &cancelled,
            serde_json::json!({
                "reason": reason,
                "status": "cancelled",
            }),
        )?;
        Ok(Some(cancelled))
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
                    .with_lease_owner("session_runner"),
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
                    SessionContinuationKind::ToolResult,
                    SessionContinuationKind::Compact,
                    SessionContinuationKind::Retry,
                ])
                .with_lease_owner("session_runner"),
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
            SessionContinuationKind::ToolResult => {
                self.run_durable_tool_result_continuation(store, continuation)
                    .await
            }
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

    pub fn enqueue_tool_result(
        &mut self,
        turn_id: TurnId,
        tool_name: impl Into<String>,
        tool_input: serde_json::Value,
    ) -> Result<SessionContinuation> {
        let store = self.continuation_store.ok_or_else(|| {
            IkarosError::Message("durable tool-result retry requires a continuation store".into())
        })?;
        let mut input = SessionContinuationInput::new(
            self.config.session_id.clone(),
            SessionContinuationKind::ToolResult,
        );
        input.turn_id = Some(turn_id);
        input.priority = SessionContinuationKind::ToolResult.default_priority();
        input.payload = serde_json::json!({
            "tool_name": tool_name.into(),
            "tool_input": tool_input,
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
                "source": "session_runner",
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
            | SessionContinuationKind::Compact
            | SessionContinuationKind::ToolResult => {
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
        let session_id = self.config.session_id.clone();
        let token = self.config.options.cancellation.clone();
        let result = {
            let mut turn_future = Box::pin(self.run_user_message(content));
            let durable_cancel = poll_durable_continuation_cancel(
                store,
                session_id.clone(),
                continuation_id.clone(),
                token,
            );
            tokio::select! {
                result = &mut turn_future => result,
                cancel_result = durable_cancel => {
                    cancel_result?;
                    turn_future.await
                }
            }
        };
        match result {
            Ok(turn) => {
                if turn.report.stop_reason == AgentLoopStopReason::Cancelled {
                    let cancelled = ensure_continuation_cancelled(
                        store,
                        &session_id,
                        &continuation_id,
                        "worker cancelled",
                    )?
                    .unwrap_or_else(|| continuation.clone());
                    self.emit_continuation_event(
                        &turn.turn_id,
                        AgentEventKind::ContinuationCancelled,
                        &cancelled,
                        serde_json::json!({
                            "acknowledged": true,
                            "completed_turn_id": turn.turn_id.as_str(),
                            "stop_reason": format!("{:?}", turn.report.stop_reason),
                        }),
                    )?;
                    return Ok(turn);
                }
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

    async fn run_durable_tool_result_continuation(
        &mut self,
        store: &dyn SessionStore,
        continuation: SessionContinuation,
    ) -> Result<AgentHarnessContinuation> {
        let continuation_id = continuation.continuation_id.clone();
        let turn_id = continuation.turn_id.clone().unwrap_or_default();
        let tool_name = continuation
            .payload
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown_tool")
            .to_string();
        let tool_input = continuation
            .payload
            .get("tool_input")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        self.emit_continuation_event(
            &turn_id,
            AgentEventKind::ContinuationStarted,
            &continuation,
            serde_json::json!({"kind": "tool_result", "tool_name": &tool_name}),
        )?;
        let started_at = time::OffsetDateTime::now_utc();
        let result = self
            .session
            .execute_skill(self.registry, &tool_name, tool_input.clone())
            .await;
        let ended_at = time::OffsetDateTime::now_utc();
        match result {
            Ok(tool_result) => {
                let result_payload =
                    tool_result_continuation_payload(&tool_result, started_at, ended_at);
                let terminal_payload =
                    serde_json::json!({"tool_name": &tool_name, "tool_result": result_payload});
                let completed = store
                    .complete_continuation(&continuation_id, terminal_payload.clone())?
                    .ok_or_else(|| {
                        IkarosError::Message(format!(
                            "continuation disappeared while completing: {continuation_id}"
                        ))
                    })?;
                self.emit_continuation_event(
                    &turn_id,
                    AgentEventKind::ContinuationCompleted,
                    &completed,
                    terminal_payload,
                )?;
                Ok(AgentHarnessContinuation::ToolResult {
                    continuation: completed,
                    turn_id,
                    tool_name,
                    result: tool_result_continuation_payload(&tool_result, started_at, ended_at),
                })
            }
            Err(error) => {
                let error_message = redact_secrets(&error.to_string());
                let failed = store
                    .fail_continuation(&continuation_id, &error_message)?
                    .unwrap_or(continuation);
                let result_payload =
                    failed_tool_result_continuation_payload(&error_message, started_at, ended_at);
                let _ = self.emit_continuation_event(
                    &turn_id,
                    AgentEventKind::ContinuationFailed,
                    &failed,
                    serde_json::json!({"tool_name": &tool_name, "tool_result": result_payload}),
                );
                Ok(AgentHarnessContinuation::ToolResult {
                    continuation: failed,
                    turn_id,
                    tool_name,
                    result: failed_tool_result_continuation_payload(
                        &error_message,
                        started_at,
                        ended_at,
                    ),
                })
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
}
