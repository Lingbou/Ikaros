// SPDX-License-Identifier: GPL-3.0-only

use super::{ChatMessageResult, ChatRunOptions, new_chat_session_id, run_chat_message};
use ikaros_core::{IkarosPaths, redact_secrets};
use ikaros_harness::CancellationToken;
use ikaros_protocol::TurnStatus;
use serde::{Deserialize, Serialize};
use std::{collections::VecDeque, path::PathBuf, sync::Arc};
use tokio::sync::{Mutex, broadcast};
use uuid::Uuid;

pub type ChatSessionRuntimeEventReceiver = broadcast::Receiver<ChatSessionRuntimeEvent>;
pub type ChatSessionRuntimeHandle = ChatSessionRuntime;

#[derive(Debug, Clone)]
pub struct ChatSessionRuntimeConfig {
    pub paths: IkarosPaths,
    pub workspace: PathBuf,
    pub agent_override: Option<String>,
    pub options: ChatRunOptions,
    pub event_capacity: usize,
}

impl ChatSessionRuntimeConfig {
    pub fn new(paths: IkarosPaths, workspace: impl Into<PathBuf>) -> Self {
        Self {
            paths,
            workspace: workspace.into(),
            agent_override: None,
            options: ChatRunOptions::default(),
            event_capacity: 256,
        }
    }

    pub fn with_agent_override(mut self, agent_override: impl Into<String>) -> Self {
        self.agent_override = Some(agent_override.into());
        self
    }

    pub fn with_options(mut self, options: ChatRunOptions) -> Self {
        self.options = options;
        self
    }

    pub fn with_event_capacity(mut self, event_capacity: usize) -> Self {
        self.event_capacity = event_capacity;
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChatSessionRuntimeEvent {
    UserSubmitted {
        session_id: String,
        turn_id: String,
        content: String,
    },
    PromptQueued {
        session_id: String,
        turn_id: String,
        queue_depth: usize,
    },
    StatusChanged {
        session_id: String,
        turn_id: Option<String>,
        status: TurnStatus,
        queue_depth: usize,
    },
    AssistantCompleted {
        session_id: String,
        turn_id: String,
        content: String,
    },
    TurnCompleted {
        session_id: String,
        turn_id: String,
        result: ChatSessionTurnResult,
    },
    TurnFailed {
        session_id: String,
        turn_id: String,
        message: String,
    },
    TurnCancelled {
        session_id: String,
        turn_id: String,
    },
    SessionCleared {
        previous_session_id: String,
        session_id: String,
    },
    SessionForked {
        source_session_id: String,
        session_id: String,
    },
    SessionResumed {
        previous_session_id: String,
        session_id: String,
    },
    ApprovalResolved {
        session_id: String,
        approval_id: String,
        approved: bool,
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub struct ChatSessionTurnResult {
    pub content: String,
    pub provider: String,
    pub model: String,
    pub streamed: bool,
    pub stream_chunks: Vec<String>,
    pub relationship_hits: usize,
    pub relationship_candidates_created: usize,
    pub reference_hits: usize,
    pub history_hits: usize,
    pub memory_hits: usize,
    pub rag_hits: usize,
    pub chat_session_id: String,
}

impl From<&ChatMessageResult> for ChatSessionTurnResult {
    fn from(result: &ChatMessageResult) -> Self {
        Self {
            content: result.content.clone(),
            provider: result.provider.clone(),
            model: result.model.clone(),
            streamed: result.streamed,
            stream_chunks: result.stream_chunks.clone(),
            relationship_hits: result.relationship_hits,
            relationship_candidates_created: result.relationship_candidates_created,
            reference_hits: result.reference_hits,
            history_hits: result.history_hits,
            memory_hits: result.memory_hits,
            rag_hits: result.rag_hits,
            chat_session_id: result.chat_session_id.clone(),
        }
    }
}

#[derive(Clone)]
pub struct ChatSessionRuntime {
    inner: Arc<RuntimeInner>,
}

struct RuntimeInner {
    config: ChatSessionRuntimeConfig,
    events: broadcast::Sender<ChatSessionRuntimeEvent>,
    state: Mutex<RuntimeState>,
}

#[derive(Debug)]
struct RuntimeState {
    session_id: String,
    previous_session_ids: Vec<String>,
    current: Option<RunningTurn>,
    pending: VecDeque<QueuedPrompt>,
    status: TurnStatus,
}

#[derive(Debug, Clone)]
struct RunningTurn {
    session_id: String,
    turn_id: String,
    cancellation: CancellationToken,
    cancel_emitted: bool,
}

#[derive(Debug, Clone)]
struct QueuedPrompt {
    session_id: String,
    turn_id: String,
    message: String,
}

impl ChatSessionRuntime {
    pub fn new(mut config: ChatSessionRuntimeConfig) -> Self {
        let session_id = config
            .options
            .session_id
            .clone()
            .unwrap_or_else(new_chat_session_id);
        config.options.session_id = Some(session_id.clone());
        let (events, _initial_receiver) = broadcast::channel(config.event_capacity.max(1));
        Self {
            inner: Arc::new(RuntimeInner {
                config,
                events,
                state: Mutex::new(RuntimeState {
                    session_id,
                    previous_session_ids: Vec::new(),
                    current: None,
                    pending: VecDeque::new(),
                    status: TurnStatus::Pending,
                }),
            }),
        }
    }

    pub fn start(self) -> ChatSessionRuntimeHandle {
        self
    }

    pub fn subscribe(&self) -> ChatSessionRuntimeEventReceiver {
        self.inner.events.subscribe()
    }

    pub async fn prompt(&self, message: impl Into<String>) -> String {
        let message = message.into();
        let turn_id = new_turn_id();
        let mut start = None;

        {
            let mut state = self.inner.state.lock().await;
            let request = QueuedPrompt {
                session_id: state.session_id.clone(),
                turn_id: turn_id.clone(),
                message,
            };
            let event_session_id = request.session_id.clone();
            let event_turn_id = request.turn_id.clone();
            self.emit(ChatSessionRuntimeEvent::UserSubmitted {
                session_id: event_session_id.clone(),
                turn_id: event_turn_id.clone(),
                content: request.message.clone(),
            });

            if state.current.is_some() {
                state.pending.push_back(request);
                let queue_depth = state.pending.len();
                self.emit(ChatSessionRuntimeEvent::PromptQueued {
                    session_id: event_session_id.clone(),
                    turn_id: event_turn_id.clone(),
                    queue_depth,
                });
                self.emit(ChatSessionRuntimeEvent::StatusChanged {
                    session_id: event_session_id,
                    turn_id: Some(event_turn_id),
                    status: state.status,
                    queue_depth,
                });
            } else {
                let cancellation = CancellationToken::new();
                state.status = TurnStatus::Running;
                state.current = Some(RunningTurn {
                    session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(),
                    cancellation: cancellation.clone(),
                    cancel_emitted: false,
                });
                self.emit(ChatSessionRuntimeEvent::StatusChanged {
                    session_id: request.session_id.clone(),
                    turn_id: Some(request.turn_id.clone()),
                    status: TurnStatus::Running,
                    queue_depth: state.pending.len(),
                });
                start = Some((request, cancellation));
            }
        }

        if let Some((request, cancellation)) = start {
            self.spawn_turn(request, cancellation);
        }

        turn_id
    }

    pub async fn abort(&self) -> bool {
        let mut events = Vec::new();
        let aborted = {
            let mut state = self.inner.state.lock().await;
            let queue_depth = state.pending.len();
            if let Some(current) = state.current.as_mut() {
                current.cancellation.cancel();
                let cancelled_turn = if !current.cancel_emitted {
                    current.cancel_emitted = true;
                    Some((current.session_id.clone(), current.turn_id.clone()))
                } else {
                    None
                };
                state.status = TurnStatus::Cancelled;
                if let Some((session_id, turn_id)) = cancelled_turn {
                    events.push(ChatSessionRuntimeEvent::TurnCancelled {
                        session_id: session_id.clone(),
                        turn_id: turn_id.clone(),
                    });
                    events.push(ChatSessionRuntimeEvent::StatusChanged {
                        session_id,
                        turn_id: Some(turn_id),
                        status: TurnStatus::Cancelled,
                        queue_depth,
                    });
                }
                true
            } else {
                false
            }
        };
        self.emit_all(events);
        aborted
    }

    pub async fn approve(&self, approval_id: impl Into<String>) {
        self.resolve_approval(approval_id.into(), true, None).await;
    }

    pub async fn deny(&self, approval_id: impl Into<String>, reason: Option<String>) {
        self.resolve_approval(approval_id.into(), false, reason)
            .await;
    }

    pub async fn clear(&self) -> String {
        self.replace_session(new_chat_session_id(), SessionReplacement::Clear)
            .await
    }

    pub async fn resume(&self, session_id: impl Into<String>) -> String {
        self.replace_session(session_id.into(), SessionReplacement::Resume)
            .await
    }

    pub async fn fork(&self) -> String {
        self.replace_session(new_chat_session_id(), SessionReplacement::Fork)
            .await
    }

    fn spawn_turn(&self, request: QueuedPrompt, cancellation: CancellationToken) {
        let runtime = self.clone();
        let handle = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            handle.block_on(async move {
                let result = runtime
                    .run_prompt_turn(&request, cancellation.clone())
                    .await;
                runtime.finish_turn(request, cancellation, result).await;
            });
        });
    }

    async fn run_prompt_turn(
        &self,
        request: &QueuedPrompt,
        cancellation: CancellationToken,
    ) -> ikaros_core::Result<ChatMessageResult> {
        let mut options = self.inner.config.options.clone();
        options.session_id = Some(request.session_id.clone());
        options.turn_id = Some(request.turn_id.clone());
        options.cancellation = cancellation;
        run_chat_message(
            &request.message,
            &self.inner.config.paths,
            &self.inner.config.workspace,
            self.inner.config.agent_override.as_deref(),
            options,
        )
        .await
    }

    async fn finish_turn(
        &self,
        request: QueuedPrompt,
        cancellation: CancellationToken,
        result: ikaros_core::Result<ChatMessageResult>,
    ) {
        let mut events = Vec::new();
        let mut next = None;

        {
            let mut state = self.inner.state.lock().await;
            let Some(current) = state.current.take() else {
                return;
            };
            if current.session_id != request.session_id || current.turn_id != request.turn_id {
                state.current = Some(current);
                return;
            }

            let queue_depth = state.pending.len();
            if cancellation.is_cancelled() {
                state.status = TurnStatus::Cancelled;
                if !current.cancel_emitted {
                    events.push(ChatSessionRuntimeEvent::TurnCancelled {
                        session_id: request.session_id.clone(),
                        turn_id: request.turn_id.clone(),
                    });
                    events.push(ChatSessionRuntimeEvent::StatusChanged {
                        session_id: request.session_id.clone(),
                        turn_id: Some(request.turn_id.clone()),
                        status: TurnStatus::Cancelled,
                        queue_depth,
                    });
                }
            } else {
                match result {
                    Ok(result) => {
                        state.status = TurnStatus::Completed;
                        events.push(ChatSessionRuntimeEvent::AssistantCompleted {
                            session_id: request.session_id.clone(),
                            turn_id: request.turn_id.clone(),
                            content: result.content.clone(),
                        });
                        events.push(ChatSessionRuntimeEvent::TurnCompleted {
                            session_id: request.session_id.clone(),
                            turn_id: request.turn_id.clone(),
                            result: ChatSessionTurnResult::from(&result),
                        });
                        events.push(ChatSessionRuntimeEvent::StatusChanged {
                            session_id: request.session_id.clone(),
                            turn_id: Some(request.turn_id.clone()),
                            status: TurnStatus::Completed,
                            queue_depth,
                        });
                    }
                    Err(error) => {
                        state.status = TurnStatus::Failed;
                        events.push(ChatSessionRuntimeEvent::TurnFailed {
                            session_id: request.session_id.clone(),
                            turn_id: request.turn_id.clone(),
                            message: redact_secrets(&error.to_string()),
                        });
                        events.push(ChatSessionRuntimeEvent::StatusChanged {
                            session_id: request.session_id.clone(),
                            turn_id: Some(request.turn_id.clone()),
                            status: TurnStatus::Failed,
                            queue_depth,
                        });
                    }
                }
            }

            if let Some(request) = state.pending.pop_front() {
                let cancellation = CancellationToken::new();
                state.status = TurnStatus::Running;
                state.current = Some(RunningTurn {
                    session_id: request.session_id.clone(),
                    turn_id: request.turn_id.clone(),
                    cancellation: cancellation.clone(),
                    cancel_emitted: false,
                });
                events.push(ChatSessionRuntimeEvent::StatusChanged {
                    session_id: request.session_id.clone(),
                    turn_id: Some(request.turn_id.clone()),
                    status: TurnStatus::Running,
                    queue_depth: state.pending.len(),
                });
                next = Some((request, cancellation));
            } else {
                state.status = TurnStatus::Pending;
            }
        }

        self.emit_all(events);
        if let Some((request, cancellation)) = next {
            self.spawn_turn(request, cancellation);
        }
    }

    async fn replace_session(
        &self,
        new_session_id: String,
        replacement: SessionReplacement,
    ) -> String {
        let mut events = Vec::new();
        let previous_session_id = {
            let mut state = self.inner.state.lock().await;
            let previous_session_id = state.session_id.clone();
            if let Some(current) = state.current.take() {
                current.cancellation.cancel();
                if !current.cancel_emitted {
                    events.push(ChatSessionRuntimeEvent::TurnCancelled {
                        session_id: current.session_id.clone(),
                        turn_id: current.turn_id.clone(),
                    });
                    events.push(ChatSessionRuntimeEvent::StatusChanged {
                        session_id: current.session_id.clone(),
                        turn_id: Some(current.turn_id.clone()),
                        status: TurnStatus::Cancelled,
                        queue_depth: state.pending.len(),
                    });
                }
            }
            state.pending.clear();
            if previous_session_id != new_session_id {
                state.previous_session_ids.push(previous_session_id.clone());
            }
            state.session_id = new_session_id.clone();
            state.status = TurnStatus::Pending;
            previous_session_id
        };

        match replacement {
            SessionReplacement::Clear => {
                events.push(ChatSessionRuntimeEvent::SessionCleared {
                    previous_session_id,
                    session_id: new_session_id.clone(),
                });
            }
            SessionReplacement::Fork => {
                events.push(ChatSessionRuntimeEvent::SessionForked {
                    source_session_id: previous_session_id,
                    session_id: new_session_id.clone(),
                });
            }
            SessionReplacement::Resume => {
                events.push(ChatSessionRuntimeEvent::SessionResumed {
                    previous_session_id,
                    session_id: new_session_id.clone(),
                });
            }
        }
        events.push(ChatSessionRuntimeEvent::StatusChanged {
            session_id: new_session_id.clone(),
            turn_id: None,
            status: TurnStatus::Pending,
            queue_depth: 0,
        });
        self.emit_all(events);
        new_session_id
    }

    async fn resolve_approval(&self, approval_id: String, approved: bool, reason: Option<String>) {
        let session_id = {
            let state = self.inner.state.lock().await;
            state.session_id.clone()
        };
        self.emit(ChatSessionRuntimeEvent::ApprovalResolved {
            session_id,
            approval_id,
            approved,
            reason,
        });
        // Future continuation seam: pass this decision back into the active turn
        // once protocol-backed turn pausing lands.
    }

    fn emit(&self, event: ChatSessionRuntimeEvent) {
        let _ignored = self.inner.events.send(event);
    }

    fn emit_all(&self, events: Vec<ChatSessionRuntimeEvent>) {
        for event in events {
            self.emit(event);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum SessionReplacement {
    Clear,
    Fork,
    Resume,
}

fn new_turn_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_runtime() -> ChatSessionRuntime {
        let home = std::env::temp_dir().join(format!(
            "ikaros-chat-session-runtime-test-{}",
            Uuid::new_v4()
        ));
        ChatSessionRuntime::new(ChatSessionRuntimeConfig::new(
            IkarosPaths::from_home(home),
            PathBuf::from("."),
        ))
    }

    #[tokio::test]
    async fn prompt_queues_when_turn_is_running() {
        let runtime = test_runtime();
        {
            let mut state = runtime.inner.state.lock().await;
            let session_id = state.session_id.clone();
            state.current = Some(RunningTurn {
                session_id,
                turn_id: "running-turn".to_owned(),
                cancellation: CancellationToken::new(),
                cancel_emitted: false,
            });
            state.status = TurnStatus::Running;
        }
        let mut receiver = runtime.subscribe();

        let queued_turn_id = runtime.prompt("queued prompt").await;

        match receiver.recv().await.expect("user submitted event") {
            ChatSessionRuntimeEvent::UserSubmitted {
                turn_id, content, ..
            } => {
                assert_eq!(turn_id, queued_turn_id);
                assert_eq!(content, "queued prompt");
            }
            event => panic!("unexpected event: {event:?}"),
        }
        match receiver.recv().await.expect("prompt queued event") {
            ChatSessionRuntimeEvent::PromptQueued {
                turn_id,
                queue_depth,
                ..
            } => {
                assert_eq!(turn_id, queued_turn_id);
                assert_eq!(queue_depth, 1);
            }
            event => panic!("unexpected event: {event:?}"),
        }
        match receiver.recv().await.expect("status changed event") {
            ChatSessionRuntimeEvent::StatusChanged {
                turn_id,
                status,
                queue_depth,
                ..
            } => {
                assert_eq!(turn_id, Some(queued_turn_id.clone()));
                assert_eq!(status, TurnStatus::Running);
                assert_eq!(queue_depth, 1);
            }
            event => panic!("unexpected event: {event:?}"),
        }

        let state = runtime.inner.state.lock().await;
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.pending[0].turn_id, queued_turn_id);
    }

    #[tokio::test]
    async fn clear_rotates_session_id_and_preserves_previous_id() {
        let runtime = test_runtime();
        let previous_session_id = {
            let state = runtime.inner.state.lock().await;
            state.session_id.clone()
        };
        let mut receiver = runtime.subscribe();

        let new_session_id = runtime.clear().await;

        assert_ne!(new_session_id, previous_session_id);
        match receiver.recv().await.expect("session cleared event") {
            ChatSessionRuntimeEvent::SessionCleared {
                previous_session_id: event_previous,
                session_id,
            } => {
                assert_eq!(event_previous, previous_session_id);
                assert_eq!(session_id, new_session_id);
            }
            event => panic!("unexpected event: {event:?}"),
        }
        match receiver.recv().await.expect("pending status event") {
            ChatSessionRuntimeEvent::StatusChanged {
                session_id,
                status,
                queue_depth,
                ..
            } => {
                assert_eq!(session_id, new_session_id);
                assert_eq!(status, TurnStatus::Pending);
                assert_eq!(queue_depth, 0);
            }
            event => panic!("unexpected event: {event:?}"),
        }

        let state = runtime.inner.state.lock().await;
        assert_eq!(state.previous_session_ids, vec![previous_session_id]);
        assert_eq!(state.session_id, new_session_id);
    }

    #[tokio::test]
    async fn subscribe_broadcasts_approval_resolution() {
        let runtime = test_runtime();
        let mut first = runtime.subscribe();
        let mut second = runtime.subscribe();

        runtime.approve("approval-1").await;

        for receiver in [&mut first, &mut second] {
            match receiver.recv().await.expect("approval event") {
                ChatSessionRuntimeEvent::ApprovalResolved {
                    approval_id,
                    approved,
                    reason,
                    ..
                } => {
                    assert_eq!(approval_id, "approval-1");
                    assert!(approved);
                    assert_eq!(reason, None);
                }
                event => panic!("unexpected event: {event:?}"),
            }
        }
    }
}
