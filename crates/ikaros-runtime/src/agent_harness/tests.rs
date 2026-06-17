// SPDX-License-Identifier: GPL-3.0-only

use super::{
    AgentHarness, AgentHarnessConfig, AgentHarnessMessage, AgentHarnessPendingCounts,
    AgentHarnessPhase,
};
use crate::agent_loop::{
    AgentLoopInput, AgentLoopOptions, AgentLoopReport, AgentLoopStopReason, AgentRuntime,
    HarnessAgentRuntime,
};
use async_trait::async_trait;
use ikaros_core::{IkarosError, Result};
use ikaros_harness::{ExecutionSession, SkillRegistry};
use ikaros_models::{MockModelProvider, ModelProvider, ModelRequest, ModelResponse, TokenUsage};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, SessionEntry, SessionEntryKind,
    SessionId, SessionStore, SqliteSessionStore, TurnId, noop_agent_event_sink,
};
use serde_json::json;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

#[derive(Default)]
struct RecordingRuntime {
    inputs: Mutex<Vec<AgentLoopInput>>,
}

#[derive(Default)]
struct SinkOnlyRuntime;

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<AgentEvent>>,
}

static TEST_SESSION_COUNTER: AtomicUsize = AtomicUsize::new(0);

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
            content: r#"{"final_answer":"called"}"#.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            diagnostics: Vec::new(),
        })
    }
}

impl RecordingRuntime {
    fn inputs(&self) -> Vec<AgentLoopInput> {
        self.inputs.lock().expect("recorded inputs").clone()
    }
}

impl AgentRuntime for RecordingRuntime {
    fn run_turn_with_events<'a>(
        &'a self,
        input: AgentLoopInput,
        provider: &'a dyn ModelProvider,
        _session: &'a ExecutionSession,
        _registry: &'a SkillRegistry,
        event_sink: &'a dyn AgentEventSink,
        _options: AgentLoopOptions,
    ) -> Pin<Box<dyn Future<Output = Result<AgentLoopReport>> + Send + 'a>> {
        Box::pin(async move {
            self.inputs
                .lock()
                .expect("record input")
                .push(input.clone());
            let session_id = input
                .session_id
                .clone()
                .map(SessionId::from)
                .ok_or_else(|| IkarosError::Message("missing session id".into()))?;
            let turn_id = input
                .turn_id
                .clone()
                .map(TurnId::from)
                .ok_or_else(|| IkarosError::Message("missing turn id".into()))?;
            let start = AgentEvent::new(
                session_id.clone(),
                turn_id.clone(),
                None,
                AgentEventSource::Runtime,
                AgentEventKind::TurnStart,
                json!({"provider": provider.name()}),
            );
            event_sink.emit(&start)?;
            let end = AgentEvent::new(
                session_id,
                turn_id,
                Some(start.event_id.clone()),
                AgentEventSource::Runtime,
                AgentEventKind::TurnEnd,
                json!({"stop_reason": "final_answer"}),
            );
            event_sink.emit(&end)?;
            Ok(AgentLoopReport {
                stop_reason: AgentLoopStopReason::FinalAnswer,
                final_content: format!("answer: {}", input.user_input),
                provider: provider.name().into(),
                model: "recording-model".into(),
                usage: TokenUsage::default(),
                streamed: false,
                stream_chunks: Vec::new(),
                iterations: 1,
                tool_call_diagnostics: Vec::new(),
                tool_results: Vec::new(),
                events: vec![start, end],
            })
        })
    }
}

impl AgentRuntime for SinkOnlyRuntime {
    fn run_turn_with_events<'a>(
        &'a self,
        input: AgentLoopInput,
        provider: &'a dyn ModelProvider,
        _session: &'a ExecutionSession,
        _registry: &'a SkillRegistry,
        event_sink: &'a dyn AgentEventSink,
        _options: AgentLoopOptions,
    ) -> Pin<Box<dyn Future<Output = Result<AgentLoopReport>> + Send + 'a>> {
        Box::pin(async move {
            let session_id = input
                .session_id
                .clone()
                .map(SessionId::from)
                .ok_or_else(|| IkarosError::Message("missing session id".into()))?;
            let turn_id = input
                .turn_id
                .clone()
                .map(TurnId::from)
                .ok_or_else(|| IkarosError::Message("missing turn id".into()))?;
            let start = AgentEvent::new(
                session_id.clone(),
                turn_id.clone(),
                None,
                AgentEventSource::Runtime,
                AgentEventKind::TurnStart,
                json!({"provider": provider.name(), "source": "sink_only"}),
            );
            event_sink.emit(&start)?;
            let end = AgentEvent::new(
                session_id,
                turn_id,
                Some(start.event_id.clone()),
                AgentEventSource::Runtime,
                AgentEventKind::TurnEnd,
                json!({"stop_reason": "final_answer", "source": "sink_only"}),
            );
            event_sink.emit(&end)?;
            Ok(AgentLoopReport {
                stop_reason: AgentLoopStopReason::FinalAnswer,
                final_content: "sink-only answer".into(),
                provider: provider.name().into(),
                model: "sink-only-model".into(),
                usage: TokenUsage::default(),
                streamed: false,
                stream_chunks: Vec::new(),
                iterations: 1,
                tool_call_diagnostics: Vec::new(),
                tool_results: Vec::new(),
                events: Vec::new(),
            })
        })
    }
}

impl RecordingSink {
    fn events(&self) -> Vec<AgentEvent> {
        self.events.lock().expect("sink events").clone()
    }
}

impl AgentEventSink for RecordingSink {
    fn emit(&self, event: &AgentEvent) -> Result<()> {
        self.events.lock().expect("sink events").push(event.clone());
        Ok(())
    }
}

fn test_session() -> ExecutionSession {
    let index = TEST_SESSION_COUNTER.fetch_add(1, Ordering::SeqCst);
    let root = std::env::temp_dir().join(format!(
        "ikaros-agent-harness-test-{}-{index}-workspace",
        std::process::id()
    ));
    let audit = std::env::temp_dir().join(format!(
        "ikaros-agent-harness-test-{}-{index}-audit",
        std::process::id()
    ));
    ExecutionSession::new(root, audit)
}

fn harness_config() -> AgentHarnessConfig {
    AgentHarnessConfig {
        session_id: SessionId::from("session-a"),
        turn_id: None,
        task_id: Some("task-a".into()),
        system_prompt: "system prompt".into(),
        options: AgentLoopOptions::default(),
    }
}

#[tokio::test]
async fn agent_harness_runs_turn_with_stable_session_and_event_first_result() {
    let runtime = RecordingRuntime::default();
    let provider = MockModelProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();
    let mut harness = AgentHarness::new(
        harness_config(),
        &runtime,
        &provider,
        &session,
        &registry,
        noop_agent_event_sink(),
    );

    assert_eq!(harness.phase(), AgentHarnessPhase::Idle);

    let turn = harness.run_turn("hello").await.expect("run harness turn");

    assert_eq!(harness.phase(), AgentHarnessPhase::Idle);
    assert_eq!(turn.report.final_content, "answer: hello");
    assert_eq!(turn.events.len(), 2);
    assert_eq!(turn.events, turn.report.events);
    assert_eq!(turn.session_id.as_str(), "session-a");
    assert!(!turn.turn_id.as_str().trim().is_empty());

    let inputs = runtime.inputs();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].session_id.as_deref(), Some("session-a"));
    assert_eq!(inputs[0].task_id.as_deref(), Some("task-a"));
    assert_eq!(inputs[0].system_prompt, "system prompt");
    assert_eq!(inputs[0].user_input, "hello");
    assert_eq!(inputs[0].turn_id.as_deref(), Some(turn.turn_id.as_str()));
}

#[tokio::test]
async fn agent_harness_continue_drains_queued_messages_by_priority() {
    let runtime = RecordingRuntime::default();
    let provider = MockModelProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();
    let mut harness = AgentHarness::new(
        harness_config(),
        &runtime,
        &provider,
        &session,
        &registry,
        noop_agent_event_sink(),
    );

    harness.enqueue_next_turn(AgentHarnessMessage::user("next"));
    harness.enqueue_follow_up(AgentHarnessMessage::user("follow-up"));
    harness.enqueue_steer(AgentHarnessMessage::user("steer"));

    assert_eq!(
        harness.pending_counts(),
        AgentHarnessPendingCounts {
            steer: 1,
            follow_up: 1,
            next_turn: 1,
        }
    );

    let steer = harness.run_continue().await.expect("steer turn");
    let follow_up = harness.run_continue().await.expect("follow-up turn");
    let next = harness.run_continue().await.expect("next turn");

    assert_eq!(steer.report.final_content, "answer: steer");
    assert_eq!(follow_up.report.final_content, "answer: follow-up");
    assert_eq!(next.report.final_content, "answer: next");
    assert_eq!(
        harness.pending_counts(),
        AgentHarnessPendingCounts::default()
    );
    assert!(harness.run_continue().await.is_err());

    let user_inputs = runtime
        .inputs()
        .into_iter()
        .map(|input| input.user_input)
        .collect::<Vec<_>>();
    assert_eq!(user_inputs, vec!["steer", "follow-up", "next"]);
}

#[tokio::test]
async fn agent_harness_can_use_caller_supplied_turn_id() {
    let runtime = RecordingRuntime::default();
    let provider = MockModelProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();
    let mut config = harness_config();
    config.turn_id = Some(TurnId::from("fixed-turn"));
    let mut harness = AgentHarness::new(
        config,
        &runtime,
        &provider,
        &session,
        &registry,
        noop_agent_event_sink(),
    );

    let turn = harness.run_turn("hello").await.expect("run harness turn");

    assert_eq!(turn.turn_id.as_str(), "fixed-turn");
    assert_eq!(runtime.inputs()[0].turn_id.as_deref(), Some("fixed-turn"));
    assert!(
        turn.events
            .iter()
            .all(|event| event.turn_id.as_str() == "fixed-turn")
    );
}

#[tokio::test]
async fn agent_harness_uses_caller_supplied_turn_id_once() {
    let runtime = RecordingRuntime::default();
    let provider = MockModelProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();
    let mut config = harness_config();
    config.turn_id = Some(TurnId::from("fixed-turn"));
    let mut harness = AgentHarness::new(
        config,
        &runtime,
        &provider,
        &session,
        &registry,
        noop_agent_event_sink(),
    );

    let first = harness.run_turn("first").await.expect("first turn");
    harness.enqueue_follow_up(AgentHarnessMessage::user("second"));
    let second = harness.run_continue().await.expect("second turn");

    assert_eq!(first.turn_id.as_str(), "fixed-turn");
    assert_ne!(second.turn_id.as_str(), "fixed-turn");
    assert_ne!(first.turn_id, second.turn_id);

    let inputs = runtime.inputs();
    assert_eq!(inputs[0].turn_id.as_deref(), Some("fixed-turn"));
    assert_eq!(inputs[1].turn_id.as_deref(), Some(second.turn_id.as_str()));
}

#[tokio::test]
async fn agent_harness_returns_emitted_events_as_report_source() {
    let runtime = SinkOnlyRuntime;
    let provider = MockModelProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();
    let sink = RecordingSink::default();
    let mut harness = AgentHarness::new(
        harness_config(),
        &runtime,
        &provider,
        &session,
        &registry,
        &sink,
    );

    let turn = harness.run_turn("hello").await.expect("run harness turn");

    assert_eq!(turn.events.len(), 2);
    assert_eq!(turn.events, sink.events());
    assert_eq!(turn.report.events, turn.events);
    assert!(matches!(turn.events[0].kind, AgentEventKind::TurnStart));
    assert!(matches!(turn.events[1].kind, AgentEventKind::TurnEnd));
}

#[tokio::test]
async fn agent_harness_cancel_aborts_next_turn_before_provider_request() {
    let runtime = HarnessAgentRuntime;
    let provider = CountingProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();
    let mut harness = AgentHarness::new(
        harness_config(),
        &runtime,
        &provider,
        &session,
        &registry,
        noop_agent_event_sink(),
    );

    harness.cancel();
    let turn = harness.run_turn("hello").await.expect("cancelled turn");

    assert_eq!(turn.report.stop_reason, AgentLoopStopReason::Cancelled);
    assert_eq!(turn.report.iterations, 0);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert_eq!(harness.phase(), AgentHarnessPhase::Idle);
}

#[test]
fn agent_harness_phase_operations_append_session_tree_entries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path().join("state"));
    let runtime = RecordingRuntime::default();
    let provider = MockModelProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();
    let mut harness = AgentHarness::new(
        harness_config(),
        &runtime,
        &provider,
        &session,
        &registry,
        noop_agent_event_sink(),
    );
    let mut root = SessionEntry::new(SessionId::from("session-a"), SessionEntryKind::UserMessage);
    root.turn_id = Some(TurnId::from("root-turn"));
    root.visible_text = Some("original user request".into());
    store.append_entry(&root).expect("root entry");

    let branch = harness
        .append_branch_summary(
            &store,
            root.entry_id.clone(),
            "try another branch",
            json!({"reason": "test"}),
        )
        .expect("branch summary");
    assert_eq!(branch.kind, SessionEntryKind::BranchSummary);
    assert_eq!(branch.parent_entry_id, Some(root.entry_id.clone()));
    assert_eq!(harness.phase(), AgentHarnessPhase::Idle);

    let compaction = harness
        .append_compaction(
            &store,
            branch.entry_id.clone(),
            "compressed prior context",
            vec![root.entry_id.clone(), branch.entry_id.clone()],
            json!({"tokens_saved": 32}),
        )
        .expect("compaction");
    assert_eq!(compaction.kind, SessionEntryKind::Compaction);
    assert_eq!(compaction.parent_entry_id, Some(branch.entry_id.clone()));
    assert_eq!(harness.phase(), AgentHarnessPhase::Idle);

    let retry = harness
        .append_retry_marker(
            &store,
            compaction.entry_id.clone(),
            Some("retry after compaction".into()),
            json!({"attempt": 2}),
        )
        .expect("retry marker");
    assert_eq!(retry.kind, SessionEntryKind::Leaf);
    assert_eq!(retry.parent_entry_id, Some(compaction.entry_id.clone()));
    assert_eq!(harness.phase(), AgentHarnessPhase::Idle);

    let branch = store
        .active_branch(&SessionId::from("session-a"))
        .expect("active branch")
        .expect("session exists");
    assert_eq!(
        branch
            .entries
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            SessionEntryKind::UserMessage,
            SessionEntryKind::BranchSummary,
            SessionEntryKind::Compaction,
            SessionEntryKind::Leaf,
        ]
    );
}
