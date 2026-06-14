// SPDX-License-Identifier: GPL-3.0-only

use super::{
    AgentEventKind, AgentLoopInput, AgentLoopOptions, AgentLoopStopReason,
    AgentLoopToolCallParseStrategy, run_agent_loop, tool_parse::parse_agent_loop_model_envelope,
};
use async_trait::async_trait;
use ikaros_core::{Result, RiskLevel};
use ikaros_harness::{
    ExecutionSession, GuardrailConfig, Skill, SkillContext, SkillOutput, SkillRegistry,
};
use ikaros_models::{
    ModelProvider, ModelRequest, ModelResponse, ModelStream, ModelStreamEvent, ModelToolCall,
    TokenUsage,
};
use ikaros_session::{PersistingAgentEventSink, SessionId, SessionStore, SqliteSessionStore};
use serde_json::json;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct SequenceProvider {
    calls: AtomicUsize,
    responses: Vec<String>,
}

#[derive(Debug)]
struct NativeToolProvider {
    calls: AtomicUsize,
}

#[derive(Debug)]
struct StreamingNativeToolProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl ModelProvider for NativeToolProvider {
    fn name(&self) -> &str {
        "native"
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            !request.tools.is_empty(),
            "agent loop should expose tool definitions to the model provider"
        );
        if index == 0 {
            return Ok(ModelResponse {
                provider: self.name().into(),
                model: "native-model".into(),
                content: String::new(),
                tool_calls: vec![ModelToolCall {
                    id: Some("call-1".into()),
                    name: "loop_echo".into(),
                    input: json!({"text": "hello token=abc123"}),
                    raw_arguments: None,
                }],
                usage: TokenUsage::default(),
            });
        }
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "native-model".into(),
            content: r#"{"final_answer":"native done"}"#.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
        })
    }
}

#[async_trait]
impl ModelProvider for SequenceProvider {
    fn name(&self) -> &str {
        "sequence"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        let content = self
            .responses
            .get(index)
            .cloned()
            .unwrap_or_else(|| "{\"final_answer\":\"done\"}".into());
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "sequence-model".into(),
            content,
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
        })
    }
}

#[async_trait]
impl ModelProvider for StreamingNativeToolProvider {
    fn name(&self) -> &str {
        "streaming-native"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        panic!("streaming-native test provider should be called through stream")
    }

    async fn stream(&self, request: ModelRequest) -> Result<ModelStream> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            !request.tools.is_empty(),
            "streaming agent loop should expose tool definitions to the model provider"
        );
        if index == 0 {
            return Ok(ModelStream {
                provider: self.name().into(),
                model: "stream-native-model".into(),
                chunks: Vec::new(),
                tool_calls: vec![ModelToolCall {
                    id: Some("call-stream".into()),
                    name: "loop_echo".into(),
                    input: json!({"text": "streamed tool token=abc123"}),
                    raw_arguments: None,
                }],
                usage: TokenUsage {
                    prompt_tokens: Some(2),
                    completion_tokens: Some(1),
                    total_tokens: Some(3),
                },
                events: Vec::new(),
            });
        }
        Ok(ModelStream {
            provider: self.name().into(),
            model: "stream-native-model".into(),
            chunks: vec![
                r#"{"final_answer":""#.into(),
                "streamed final token=abc123".into(),
                r#""}"#.into(),
            ],
            tool_calls: Vec::new(),
            usage: TokenUsage {
                prompt_tokens: Some(3),
                completion_tokens: Some(4),
                total_tokens: Some(7),
            },
            events: Vec::new(),
        })
    }
}

#[derive(Debug)]
struct EchoSkill {
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Skill for EchoSkill {
    fn name(&self) -> &'static str {
        "loop_echo"
    }

    fn description(&self) -> &'static str {
        "echoes input"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(SkillOutput::new("echo ok", json!({"input": input})))
    }
}

#[derive(Debug)]
struct NoProgressSkill;

#[async_trait]
impl Skill for NoProgressSkill {
    fn name(&self) -> &'static str {
        "loop_no_progress"
    }

    fn description(&self) -> &'static str {
        "returns no progress"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        Ok(SkillOutput::new("no progress", json!({"progress": false})))
    }
}

#[tokio::test]
async fn agent_loop_dispatches_tool_then_finishes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = SkillRegistry::new();
    registry.register(EchoSkill {
        calls: calls.clone(),
    });
    let provider = SequenceProvider {
        calls: AtomicUsize::new(0),
        responses: vec![
            r#"{"tool_calls":[{"name":"loop_echo","input":{"text":"hello token=abc123"}}]}"#.into(),
            r#"{"final_answer":"finished token=abc123"}"#.into(),
        ],
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("loop-task".into()),
            task_id: Some("loop-task".into()),
            system_prompt: "Use tools when useful.".into(),
            user_input: "start token=abc123".into(),
        },
        &provider,
        &session,
        &registry,
        AgentLoopOptions::default(),
    )
    .await
    .expect("agent loop");

    assert_eq!(report.stop_reason, AgentLoopStopReason::FinalAnswer);
    assert_eq!(report.iterations, 2);
    assert_eq!(report.tool_results.len(), 1);
    assert_eq!(report.tool_call_diagnostics.len(), 2);
    assert_eq!(
        report.tool_call_diagnostics[0].strategy,
        AgentLoopToolCallParseStrategy::JsonFallback
    );
    assert!(!report.tool_call_diagnostics[0].repaired);
    assert_eq!(report.tool_call_diagnostics[0].tool_call_count, 1);
    assert_eq!(
        report.tool_call_diagnostics[1].strategy,
        AgentLoopToolCallParseStrategy::JsonFallback
    );
    assert!(report.tool_call_diagnostics[1].has_final_answer);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(report.final_content.contains("[REDACTED_SECRET]"));
    assert!(!report.final_content.contains("abc123"));
    let events = session.audit.read_all().expect("audit");
    assert!(events.iter().any(|event| event.kind == "agent_loop_start"));
    assert!(
        events
            .iter()
            .any(|event| event.kind == "agent_loop_model_result")
    );
    assert!(events.iter().any(|event| event.kind == "agent_loop_end"));
}

#[tokio::test]
async fn agent_loop_halts_on_guardrail_no_progress() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(NoProgressSkill);
    let provider = SequenceProvider {
        calls: AtomicUsize::new(0),
        responses: vec![
            r#"{"tool_calls":[{"name":"loop_no_progress","input":{}}]}"#.into(),
            r#"{"tool_calls":[{"name":"loop_no_progress","input":{}}]}"#.into(),
        ],
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("loop-guardrail".into()),
            task_id: Some("loop-guardrail".into()),
            system_prompt: "Use tools when useful.".into(),
            user_input: "start".into(),
        },
        &provider,
        &session,
        &registry,
        AgentLoopOptions {
            guardrails: GuardrailConfig {
                hard_stop_enabled: true,
                no_progress_halt_after: 2,
                no_progress_warn_after: 10,
                ..GuardrailConfig::default()
            },
            ..AgentLoopOptions::default()
        },
    )
    .await
    .expect("agent loop");

    assert_eq!(report.stop_reason, AgentLoopStopReason::GuardrailHalt);
    assert_eq!(report.iterations, 2);
    assert_eq!(report.tool_results.len(), 2);
    let events = session.audit.read_all().expect("audit");
    assert!(
        events
            .iter()
            .any(|event| event.kind == "agent_loop_guardrail_halt")
    );
}

#[tokio::test]
async fn agent_loop_dispatches_provider_native_tool_calls() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = SkillRegistry::new();
    registry.register(EchoSkill {
        calls: calls.clone(),
    });
    let provider = NativeToolProvider {
        calls: AtomicUsize::new(0),
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("native-loop".into()),
            task_id: Some("native-loop".into()),
            system_prompt: "Use native tools when useful.".into(),
            user_input: "start".into(),
        },
        &provider,
        &session,
        &registry,
        AgentLoopOptions::default(),
    )
    .await
    .expect("agent loop");

    assert_eq!(report.stop_reason, AgentLoopStopReason::FinalAnswer);
    assert_eq!(report.tool_results.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(report.tool_results[0].name, "loop_echo");
    assert_eq!(
        report.tool_call_diagnostics[0].strategy,
        AgentLoopToolCallParseStrategy::ProviderNativeToolCalls
    );
    assert!(!report.tool_call_diagnostics[0].repaired);
    assert_eq!(
        report.tool_results[0].output["input"]["text"],
        "hello token=[REDACTED_SECRET]"
    );
    let events = session.audit.read_all().expect("audit");
    assert!(events.iter().any(|event| {
        event.kind == "agent_loop_model_result"
            && event
                .data
                .get("native_tool_call_count")
                .and_then(serde_json::Value::as_u64)
                == Some(1)
            && event
                .data
                .get("parse_strategy")
                .and_then(serde_json::Value::as_str)
                == Some("provider_native_tool_calls")
            && event
                .data
                .get("repaired")
                .and_then(serde_json::Value::as_bool)
                == Some(false)
    }));
}

#[tokio::test]
async fn agent_loop_streams_final_answer_after_streamed_tool_call() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = SkillRegistry::new();
    registry.register(EchoSkill {
        calls: calls.clone(),
    });
    let provider = StreamingNativeToolProvider {
        calls: AtomicUsize::new(0),
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("stream-loop".into()),
            task_id: Some("stream-loop".into()),
            system_prompt: "Use tools when useful.".into(),
            user_input: "start".into(),
        },
        &provider,
        &session,
        &registry,
        AgentLoopOptions {
            stream: true,
            ..AgentLoopOptions::default()
        },
    )
    .await
    .expect("agent loop");

    assert_eq!(report.stop_reason, AgentLoopStopReason::FinalAnswer);
    assert!(report.streamed);
    assert_eq!(report.iterations, 2);
    assert_eq!(report.tool_results.len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        report.tool_call_diagnostics[0].strategy,
        AgentLoopToolCallParseStrategy::ProviderNativeToolCalls
    );
    assert_eq!(
        report.tool_call_diagnostics[1].strategy,
        AgentLoopToolCallParseStrategy::JsonFallback
    );
    assert_eq!(
        report.final_content,
        "streamed final token=[REDACTED_SECRET]"
    );
    let streamed = report.stream_chunks.join("");
    assert_eq!(streamed, report.final_content);
    assert!(!streamed.contains("final_answer"));
    assert!(!streamed.contains("abc123"));
    assert!(streamed.contains("[REDACTED_SECRET]"));
    assert_eq!(report.usage.total_tokens, Some(10));
    assert!(matches!(
        report.events.first().map(|event| &event.kind),
        Some(AgentEventKind::SessionStart)
    ));
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::TurnStart))
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolStart))
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolEnd))
    );
    assert!(report.events.iter().any(|event| {
        matches!(
            &event.kind,
            AgentEventKind::ModelStream(ModelStreamEvent::ToolCallStart { name, .. })
                if name == "loop_echo"
        )
    }));
    assert!(matches!(
        report.events.last().map(|event| &event.kind),
        Some(AgentEventKind::TurnEnd)
    ));
    let events = session.audit.read_all().expect("audit");
    assert!(events.iter().any(|event| {
        event.kind == "agent_loop_model_result"
            && event
                .data
                .get("streamed")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            && event
                .data
                .get("native_tool_call_count")
                .and_then(serde_json::Value::as_u64)
                == Some(1)
    }));
    assert!(events.iter().any(|event| {
        event.kind == "agent_loop_end"
            && event
                .data
                .get("streamed")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            && event
                .data
                .get("stream_chunk_count")
                .and_then(serde_json::Value::as_u64)
                == Some(1)
    }));
}

#[tokio::test]
async fn agent_loop_can_persist_event_timeline_to_session_store() {
    let temp = tempfile::tempdir().expect("tempdir");
    let execution = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let session_store = Arc::new(SqliteSessionStore::new(temp.path().join("state")));
    let event_sink = PersistingAgentEventSink::new(session_store.clone()).with_agent_id("build");
    let mut registry = SkillRegistry::new();
    registry.register(EchoSkill {
        calls: Arc::new(AtomicUsize::new(0)),
    });
    let provider = NativeToolProvider {
        calls: AtomicUsize::new(0),
    };

    let report = super::run_agent_loop_with_events(
        AgentLoopInput {
            session_id: Some("persist-loop".into()),
            task_id: Some("persist-loop".into()),
            system_prompt: "Use native tools when useful.".into(),
            user_input: "start token=abc123".into(),
        },
        &provider,
        &execution,
        &registry,
        &event_sink,
        AgentLoopOptions::default(),
    )
    .await
    .expect("agent loop");

    let replay = session_store
        .replay_session(&SessionId::from("persist-loop"))
        .expect("replay")
        .expect("session exists");
    assert_eq!(replay.session.agent_id.as_deref(), Some("build"));
    assert_eq!(replay.agent_events, report.events);
    assert!(matches!(
        replay.agent_events.first().map(|event| &event.kind),
        Some(AgentEventKind::SessionStart)
    ));
    assert!(matches!(
        replay.agent_events.last().map(|event| &event.kind),
        Some(AgentEventKind::TurnEnd)
    ));
    let replay_json = serde_json::to_string(&replay).expect("json");
    assert!(!replay_json.contains("abc123"));
    assert!(replay_json.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn agent_loop_without_session_id_uses_fresh_session_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let registry = SkillRegistry::new();

    let first = run_agent_loop(
        AgentLoopInput {
            session_id: None,
            task_id: None,
            system_prompt: "Answer directly.".into(),
            user_input: "first".into(),
        },
        &SequenceProvider {
            calls: AtomicUsize::new(0),
            responses: vec![r#"{"final_answer":"first done"}"#.into()],
        },
        &session,
        &registry,
        AgentLoopOptions::default(),
    )
    .await
    .expect("first loop");
    let second = run_agent_loop(
        AgentLoopInput {
            session_id: None,
            task_id: None,
            system_prompt: "Answer directly.".into(),
            user_input: "second".into(),
        },
        &SequenceProvider {
            calls: AtomicUsize::new(0),
            responses: vec![r#"{"final_answer":"second done"}"#.into()],
        },
        &session,
        &registry,
        AgentLoopOptions::default(),
    )
    .await
    .expect("second loop");

    let first_session = &first.events[0].session_id;
    let second_session = &second.events[0].session_id;
    assert_ne!(first_session.as_str(), "local");
    assert_ne!(second_session.as_str(), "local");
    assert_ne!(first_session, second_session);
}

#[test]
fn parses_only_canonical_json_tool_call_fallback() {
    let envelope = parse_agent_loop_model_envelope(
        r#"{"tool_calls":[{"id":"call_1","name":"loop_echo","input":{"text":"hello token=abc123"}}]}"#,
    )
    .expect("canonical tool call");
    assert_eq!(envelope.tool_calls.len(), 1);
    assert_eq!(
        envelope.parse_strategy,
        Some(AgentLoopToolCallParseStrategy::JsonFallback)
    );
    assert_eq!(envelope.tool_calls[0].id.as_deref(), Some("call_1"));
    assert_eq!(envelope.tool_calls[0].name, "loop_echo");
    assert_eq!(
        envelope.tool_calls[0].input["text"],
        "hello token=[REDACTED_SECRET]"
    );

    assert!(
        parse_agent_loop_model_envelope(
            r#"{"tool_calls":[{"function":{"name":"loop_echo","arguments":"{\"text\":\"hi\"}"}}]}"#,
        )
        .is_none()
    );
    assert!(
        parse_agent_loop_model_envelope(
            r#"Use this:
```json
[{"name":"loop_echo","args":"{\"text\":\"hello\"}"}]
```"#,
        )
        .is_none()
    );
    assert!(
        parse_agent_loop_model_envelope(
            r#"I will call this tool: {"tool_call":{"name":"loop_echo","args":{"text":"embedded"}}}"#,
        )
        .is_none()
    );
}
