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
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, SessionId, TurnId,
    noop_agent_event_sink,
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
