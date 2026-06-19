// SPDX-License-Identifier: GPL-3.0-only

use super::{
    AgentHarness, AgentHarnessConfig, AgentHarnessContinuation, AgentHarnessMessage,
    AgentHarnessPendingCounts, AgentHarnessPhase,
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
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, PersistingAgentEventSink,
    PersistingAgentTurnSink, SessionContinuationStatus, SessionEntry, SessionEntryKind, SessionId,
    SessionSource, SessionStore, SqliteSessionStore, TurnId, noop_agent_event_sink,
};
use serde_json::json;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
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

#[derive(Debug, Default)]
struct SlowProvider {
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

#[async_trait]
impl ModelProvider for SlowProvider {
    fn name(&self) -> &str {
        "slow"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "slow-model".into(),
            content: r#"{"final_answer":"too late"}"#.into(),
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

    harness
        .enqueue_next_turn(AgentHarnessMessage::user("next"))
        .expect("next");
    harness
        .enqueue_follow_up(AgentHarnessMessage::user("follow-up"))
        .expect("follow-up");
    harness
        .enqueue_steer(AgentHarnessMessage::user("steer"))
        .expect("steer");

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
    harness
        .enqueue_follow_up(AgentHarnessMessage::user("second"))
        .expect("follow-up");
    let second = harness.run_continue().await.expect("second turn");

    assert_eq!(first.turn_id.as_str(), "fixed-turn");
    assert_ne!(second.turn_id.as_str(), "fixed-turn");
    assert_ne!(first.turn_id, second.turn_id);

    let inputs = runtime.inputs();
    assert_eq!(inputs[0].turn_id.as_deref(), Some("fixed-turn"));
    assert_eq!(inputs[1].turn_id.as_deref(), Some(second.turn_id.as_str()));
}

#[tokio::test]
async fn agent_harness_persists_continuations_across_harness_instances() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path().join("state"));
    let runtime = RecordingRuntime::default();
    let provider = MockModelProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();

    {
        let mut harness = AgentHarness::new(
            harness_config(),
            &runtime,
            &provider,
            &session,
            &registry,
            noop_agent_event_sink(),
        )
        .with_continuation_store(&store);
        harness
            .enqueue_follow_up(AgentHarnessMessage::user("persisted follow-up"))
            .expect("enqueue");
    }

    let mut restarted = AgentHarness::new(
        harness_config(),
        &runtime,
        &provider,
        &session,
        &registry,
        noop_agent_event_sink(),
    )
    .with_continuation_store(&store);
    let turn = restarted.run_continue().await.expect("run persisted");

    assert_eq!(turn.report.final_content, "answer: persisted follow-up");
    assert_eq!(runtime.inputs()[0].user_input, "persisted follow-up");
    let continuations = store
        .continuations(&SessionId::from("session-a"))
        .expect("continuations");
    assert_eq!(continuations.len(), 1);
    assert_eq!(
        continuations[0].status,
        SessionContinuationStatus::Completed
    );
    assert_eq!(
        continuations[0].payload["completed_turn_id"],
        json!(turn.turn_id.as_str())
    );
}

#[tokio::test]
async fn agent_harness_runs_persisted_continuation_with_turn_sink_without_locking() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path().join("state"));
    let sink_store: std::sync::Arc<dyn SessionStore> = std::sync::Arc::new(store.clone());
    let event_sink = PersistingAgentTurnSink::new(sink_store)
        .with_source(SessionSource::Test)
        .with_agent_id("build");
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
        &event_sink,
    )
    .with_continuation_store(&store);

    harness
        .enqueue_follow_up(AgentHarnessMessage::user("persisted sink follow-up"))
        .expect("enqueue");
    let turn = harness.run_continue().await.expect("run continuation");
    event_sink.commit().expect("commit turn events");

    let continuations = store
        .continuations(&SessionId::from("session-a"))
        .expect("continuations");
    assert_eq!(
        continuations[0].status,
        SessionContinuationStatus::Completed
    );
    let replay = store
        .replay_session(&SessionId::from("session-a"))
        .expect("replay")
        .expect("session");
    assert!(replay.agent_events.iter().any(|event| {
        event.turn_id == turn.turn_id && matches!(event.kind, AgentEventKind::ContinuationStarted)
    }));
    assert!(replay.agent_events.iter().any(|event| {
        event.turn_id == turn.turn_id && matches!(event.kind, AgentEventKind::ContinuationCompleted)
    }));
}

#[test]
fn agent_harness_cancels_durable_continuation_with_replay_event() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path().join("state"));
    let event_store: std::sync::Arc<dyn SessionStore> = std::sync::Arc::new(store.clone());
    let event_sink = PersistingAgentEventSink::new(event_store).with_source(SessionSource::Test);
    let runtime = RecordingRuntime::default();
    let provider = MockModelProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();
    let mut config = harness_config();
    config.turn_id = Some(TurnId::from("cancel-turn"));
    let mut harness = AgentHarness::new(
        config,
        &runtime,
        &provider,
        &session,
        &registry,
        &event_sink,
    )
    .with_continuation_store(&store);

    harness
        .enqueue_follow_up(AgentHarnessMessage::user("queued follow-up"))
        .expect("enqueue");
    let queued = store
        .continuations(&SessionId::from("session-a"))
        .expect("continuations")
        .into_iter()
        .next()
        .expect("queued continuation");

    let cancelled = harness
        .cancel_continuation(&queued.continuation_id, "operator cancelled")
        .expect("cancel")
        .expect("cancelled");

    assert_eq!(cancelled.status, SessionContinuationStatus::Cancelled);
    assert_eq!(cancelled.error.as_deref(), Some("operator cancelled"));
    let replay = store
        .replay_session(&SessionId::from("session-a"))
        .expect("replay")
        .expect("session");
    let event = replay
        .agent_events
        .iter()
        .find(|event| matches!(event.kind, AgentEventKind::ContinuationCancelled))
        .expect("cancel event");
    assert_eq!(event.turn_id.as_str(), "cancel-turn");
    assert_eq!(
        event.payload["continuation_id"],
        json!(queued.continuation_id.as_str())
    );
    assert_eq!(
        event.payload["payload"]["reason"],
        json!("operator cancelled")
    );
}

#[tokio::test]
async fn running_durable_continuation_observes_external_cancel() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path().join("state"));
    let event_store: std::sync::Arc<dyn SessionStore> = std::sync::Arc::new(store.clone());
    let event_sink = PersistingAgentEventSink::new(event_store).with_source(SessionSource::Test);
    let runtime = HarnessAgentRuntime;
    let provider = SlowProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();
    let mut worker = AgentHarness::new(
        harness_config(),
        &runtime,
        &provider,
        &session,
        &registry,
        &event_sink,
    )
    .with_continuation_store(&store);
    worker
        .enqueue_follow_up(AgentHarnessMessage::user("slow follow-up"))
        .expect("enqueue");
    let continuation_id = store
        .continuations(&SessionId::from("session-a"))
        .expect("continuations")
        .into_iter()
        .next()
        .expect("queued continuation")
        .continuation_id;

    let mut running = Box::pin(worker.run_continue());
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(50)) => {}
        result = &mut running => panic!("continuation finished before external cancel: {result:?}"),
    }

    let canceller = AgentHarness::new(
        harness_config(),
        &runtime,
        &provider,
        &session,
        &registry,
        &event_sink,
    )
    .with_continuation_store(&store);
    canceller
        .cancel_continuation(&continuation_id, "external abort")
        .expect("cancel")
        .expect("cancelled");

    let turn = tokio::time::timeout(Duration::from_secs(2), &mut running)
        .await
        .expect("worker should observe durable cancellation")
        .expect("cancelled turn");
    assert_eq!(turn.report.stop_reason, AgentLoopStopReason::Cancelled);

    let continuations = store
        .continuations(&SessionId::from("session-a"))
        .expect("continuations");
    assert_eq!(
        continuations[0].status,
        SessionContinuationStatus::Cancelled
    );
    assert_eq!(continuations[0].error.as_deref(), Some("external abort"));

    let replay = store
        .replay_session(&SessionId::from("session-a"))
        .expect("replay")
        .expect("session");
    let cancel_events = replay
        .agent_events
        .iter()
        .filter(|event| matches!(event.kind, AgentEventKind::ContinuationCancelled))
        .collect::<Vec<_>>();
    assert!(
        cancel_events
            .iter()
            .any(|event| event.payload["payload"]["reason"] == json!("external abort"))
    );
    assert!(
        cancel_events
            .iter()
            .any(|event| event.payload["payload"]["acknowledged"] == json!(true))
    );
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

#[tokio::test]
async fn agent_harness_runs_retry_and_compaction_as_durable_continuations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path().join("state"));
    let runtime = RecordingRuntime::default();
    let provider = MockModelProvider::default();
    let session = test_session();
    let registry = SkillRegistry::new();
    let sink = RecordingSink::default();
    let mut root = SessionEntry::new(SessionId::from("session-a"), SessionEntryKind::UserMessage);
    root.turn_id = Some(TurnId::from("root-turn"));
    root.visible_text = Some("original user request".into());
    store.append_entry(&root).expect("root entry");

    let mut harness = AgentHarness::new(
        harness_config(),
        &runtime,
        &provider,
        &session,
        &registry,
        &sink,
    )
    .with_continuation_store(&store);
    let compacted = harness
        .enqueue_compaction(
            root.entry_id.clone(),
            "compressed old branch",
            vec![root.entry_id.clone()],
            json!({"tokens_saved": 64}),
        )
        .expect("enqueue compaction");
    let compaction_result = harness
        .run_next_continuation()
        .await
        .expect("run compaction");
    let AgentHarnessContinuation::Entry {
        continuation: completed_compaction,
        entry: compaction_entry,
    } = compaction_result
    else {
        panic!("expected compaction entry continuation");
    };
    assert_eq!(
        completed_compaction.continuation_id,
        compacted.continuation_id
    );
    assert_eq!(
        completed_compaction.status,
        SessionContinuationStatus::Completed
    );
    assert_eq!(compaction_entry.kind, SessionEntryKind::Compaction);

    let retry = harness
        .enqueue_retry_marker(
            compaction_entry.entry_id.clone(),
            Some("retry after evidence".into()),
            json!({"attempt": 2}),
        )
        .expect("enqueue retry");

    let retry_result = harness.run_next_continuation().await.expect("run retry");
    let AgentHarnessContinuation::Entry {
        continuation: completed_retry,
        entry: retry_entry,
    } = retry_result
    else {
        panic!("expected retry entry continuation");
    };
    assert_eq!(completed_retry.continuation_id, retry.continuation_id);
    assert_eq!(completed_retry.status, SessionContinuationStatus::Completed);
    assert_eq!(retry_entry.kind, SessionEntryKind::Leaf);

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
            SessionEntryKind::Compaction,
            SessionEntryKind::Leaf,
        ]
    );
    let event_kinds = sink
        .events()
        .iter()
        .map(|event| event.kind.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        event_kinds,
        vec![
            AgentEventKind::ContinuationStarted,
            AgentEventKind::ContinuationCompleted,
            AgentEventKind::ContinuationStarted,
            AgentEventKind::ContinuationCompleted,
        ]
    );
}
