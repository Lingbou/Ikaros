// SPDX-License-Identifier: GPL-3.0-only

use super::prompt::build_chat_system_prompt;
use super::*;
use crate::noop_agent_event_sink;
use async_trait::async_trait;
use ikaros_context::{
    HeuristicTokenEstimator, PromptRedactionState, PromptSectionKind, PromptSourceKind,
    TokenEstimator, apply_context_token_budget, chat_context_token_count,
};
use ikaros_core::{
    AgentProfile, ContextBuilder, IkarosError, IkarosPaths, RagConfig, ResolvedAgentProfile, Result,
};
use ikaros_harness::{
    CancellationToken, GovernedNetworkEgress, LocalExecutionEnv, NetworkEgress,
    NetworkEgressPolicy, NetworkEgressRequest, NetworkEgressResponse, NetworkedExecutionEnv,
};
use ikaros_memory::{JsonlMemoryCandidateStore, MemoryCandidateStatus};
use ikaros_models::{
    ModelContextProfile, ModelProvider, ModelRequest, ModelRequestOptions, ModelResponse,
    ModelStreamEvent, ModelTokenizerKind,
};
use ikaros_session::{
    AgentEventKind, PersistingAgentTurnSink, SessionEntry, SessionEntryKind, SessionId,
    SessionInputStatus, SessionRecord, SessionReplay, SessionSource, SessionStore,
    SqliteSessionStore, TurnId,
};
use ikaros_soul::{EmotionState, PersonaLoader};
use std::{
    collections::BTreeMap,
    fs,
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Debug)]
struct FailingProvider;

#[async_trait]
impl ModelProvider for FailingProvider {
    fn name(&self) -> &str {
        "failing"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        Err(IkarosError::Message(
            "provider failed with token=abc123".into(),
        ))
    }
}

#[derive(Debug, Default)]
struct CountingProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl ModelProvider for CountingProvider {
    fn name(&self) -> &str {
        "counting"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "counting-model".into(),
            content: "should not be called".into(),
            tool_calls: Vec::new(),
            usage: Default::default(),
            diagnostics: Vec::new(),
        })
    }
}

#[derive(Debug, Default)]
struct RecordingOptionsProvider {
    max_tokens: Mutex<Option<u32>>,
}

#[async_trait]
impl ModelProvider for RecordingOptionsProvider {
    fn name(&self) -> &str {
        "recording-options"
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        *self.max_tokens.lock().expect("max tokens") = request.options.max_tokens;
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "recording-options-model".into(),
            content: r#"{"final_answer":"ok"}"#.into(),
            tool_calls: Vec::new(),
            usage: Default::default(),
            diagnostics: Vec::new(),
        })
    }
}

#[derive(Debug, Default)]
struct RecordingMessagesProvider {
    messages: Mutex<Vec<ikaros_models::ModelMessage>>,
}

#[async_trait]
impl ModelProvider for RecordingMessagesProvider {
    fn name(&self) -> &str {
        "recording-messages"
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        *self.messages.lock().expect("messages") = request.messages;
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "recording-messages-model".into(),
            content: r#"{"final_answer":"ok"}"#.into(),
            tool_calls: Vec::new(),
            usage: Default::default(),
            diagnostics: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct TinyWindowProvider;

#[async_trait]
impl ModelProvider for TinyWindowProvider {
    fn name(&self) -> &str {
        "tiny-window"
    }

    fn context_profile(&self) -> ModelContextProfile {
        ModelContextProfile::new(96, 32, ModelTokenizerKind::Mock, "tiny-window")
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "tiny-window-model".into(),
            content: "ok".into(),
            tool_calls: Vec::new(),
            usage: Default::default(),
            diagnostics: Vec::new(),
        })
    }
}

#[derive(Debug, Default)]
struct ProviderBackedSummaryProvider {
    summary_calls: AtomicUsize,
}

#[async_trait]
impl ModelProvider for ProviderBackedSummaryProvider {
    fn name(&self) -> &str {
        "provider-summary"
    }

    fn context_profile(&self) -> ModelContextProfile {
        ModelContextProfile::new(96, 32, ModelTokenizerKind::Mock, "provider-summary")
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let is_summary_request = request
            .messages
            .iter()
            .any(|message| message.content.contains("Summarize local context"));
        if is_summary_request {
            self.summary_calls.fetch_add(1, Ordering::SeqCst);
            return Ok(ModelResponse {
                provider: self.name().into(),
                model: "provider-summary-model".into(),
                content: "provider summary keeps old turn facts token=sk-secret-value".into(),
                tool_calls: Vec::new(),
                usage: Default::default(),
                diagnostics: Vec::new(),
            });
        }
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "provider-summary-model".into(),
            content: "ok".into(),
            tool_calls: Vec::new(),
            usage: Default::default(),
            diagnostics: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct FixtureNetworkEgress;

impl NetworkEgress for FixtureNetworkEgress {
    fn send_network_request<'a>(
        &'a self,
        request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        Box::pin(async move {
            Ok(NetworkEgressResponse {
                status: 200,
                headers: Default::default(),
                body: format!("fetched {} with remote docs token=abc123", request.url),
                body_bytes: None,
            })
        })
    }
}

#[derive(Debug)]
struct FixedNetworkEgress {
    response: NetworkEgressResponse,
}

impl NetworkEgress for FixedNetworkEgress {
    fn send_network_request<'a>(
        &'a self,
        _request: NetworkEgressRequest,
    ) -> Pin<Box<dyn Future<Output = Result<NetworkEgressResponse>> + Send + 'a>> {
        Box::pin(async move { Ok(self.response.clone()) })
    }
}

mod context;
mod history;
mod prompt;
mod turn;

fn push_history_record_as_replay_entries(
    replays: &mut Vec<SessionReplay>,
    record: ChatHistoryRecord,
) {
    let replay_index = replays
        .iter()
        .position(|replay| replay.session.session_id.as_str() == record.session_id)
        .unwrap_or_else(|| {
            replays.push(SessionReplay {
                session: SessionRecord::new(
                    SessionId::from(record.session_id.clone()),
                    SessionSource::Test,
                ),
                entries: Vec::new(),
                agent_events: Vec::new(),
                approvals: Vec::new(),
            });
            replays.len() - 1
        });
    let replay = &mut replays[replay_index];
    let turn_index = replay
        .entries
        .iter()
        .filter(|entry| entry.kind == SessionEntryKind::AssistantMessage)
        .count() as i64;
    let at = time::OffsetDateTime::UNIX_EPOCH + time::Duration::seconds(turn_index + 1);
    let session_id = replay.session.session_id.clone();
    let turn_id = TurnId::from(record.turn_id.clone());
    let parent_entry_id = replay.entries.last().map(|entry| entry.entry_id.clone());

    let mut user = SessionEntry::new(session_id.clone(), SessionEntryKind::UserMessage);
    user.parent_entry_id = parent_entry_id;
    user.turn_id = Some(turn_id.clone());
    user.at = at;
    user.visible_text = Some(record.user_message.clone());
    user.payload = serde_json::json!({
        "role": "user",
        "content": record.user_message,
    });
    let mut assistant = SessionEntry::new(session_id, SessionEntryKind::AssistantMessage);
    assistant.parent_entry_id = Some(user.entry_id.clone());
    assistant.turn_id = Some(turn_id);
    assistant.at = at + time::Duration::milliseconds(1);
    assistant.visible_text = Some(record.assistant_message.clone());
    assistant.payload = serde_json::json!({
        "role": "assistant",
        "agent": record.agent,
        "provider": record.provider,
        "model": record.model,
        "streamed": record.streamed,
        "content": record.assistant_message,
        "relationship_hits": record.relationship_hits,
        "memory_hits": record.memory_hits,
        "rag_hits": record.rag_hits,
    });

    replay.entries.push(user);
    replay.entries.push(assistant);
}

fn chat_history_record(session_id: &str, user_message: &str) -> ChatHistoryRecord {
    ChatHistoryRecord {
        session_id: ikaros_core::redact_secrets(session_id),
        turn_id: uuid::Uuid::new_v4().to_string(),
        created_at: ikaros_core::now_rfc3339().expect("timestamp"),
        agent: "build".into(),
        provider: "mock".into(),
        model: "mock-ikaros".into(),
        streamed: false,
        user_message: ikaros_core::redact_secrets(user_message),
        assistant_message: "stored safely".into(),
        relationship_hits: 0,
        memory_hits: 0,
        rag_hits: 0,
    }
}

fn write_offline_mock_config(paths: &IkarosPaths) {
    fs::create_dir_all(&paths.home).expect("home");
    fs::write(
        &paths.config,
        r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
    voice: default
  asr:
    provider: mock
    model: mock-asr
"#,
    )
    .expect("mock config");
}
