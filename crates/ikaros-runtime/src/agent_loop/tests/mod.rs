// SPDX-License-Identifier: GPL-3.0-only

use super::{
    AgentEventKind, AgentEventSource, AgentLoopHookEvent, AgentLoopHooks, AgentLoopInput,
    AgentLoopOptions, AgentLoopStopReason, AgentLoopToolCallParseStrategy, AgentRuntime,
    HarnessAgentRuntime, RecordingAgentRuntime, prompt::build_agent_loop_system_prompt,
    run_agent_loop, tool_parse::parse_agent_loop_model_envelope,
};
use async_trait::async_trait;
use ikaros_context::{HeuristicTokenEstimator, PromptSectionKind, PromptSourceKind};
use ikaros_core::{IkarosError, IkarosPaths, Result, RiskLevel};
use ikaros_harness::{
    CancellationToken, ExecutionSession, GuardrailConfig, Skill, SkillContext, SkillDescriptor,
    SkillOutput, SkillRegistry, ToolExecutionMode, Toolset, ToolsetSelection,
};
use ikaros_models::{
    ModelProvider, ModelRequest, ModelRequestDiagnostic, ModelResponse, ModelStream,
    ModelStreamEvent, ModelToolCall, TokenUsage,
};
use ikaros_session::{
    PersistingAgentEventSink, PersistingAgentTurnSink, SessionId, SessionStore, SqliteSessionStore,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::time::{Duration, sleep};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    Layer, Registry,
    layer::{Context as TraceLayerContext, SubscriberExt},
};

#[derive(Debug, Clone, Default)]
struct RecordingTracingLayer {
    events: Arc<Mutex<Vec<RecordedTracingEvent>>>,
}

#[derive(Debug, Clone, Default)]
struct RecordedTracingEvent {
    target: String,
    name: String,
    fields: BTreeMap<String, String>,
}

impl RecordingTracingLayer {
    fn events(&self) -> Arc<Mutex<Vec<RecordedTracingEvent>>> {
        self.events.clone()
    }
}

impl<S> Layer<S> for RecordingTracingLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: TraceLayerContext<'_, S>) {
        let mut visitor = RecordingFieldVisitor::default();
        event.record(&mut visitor);
        self.events
            .lock()
            .expect("trace events")
            .push(RecordedTracingEvent {
                target: event.metadata().target().to_owned(),
                name: event.metadata().name().to_owned(),
                fields: visitor.fields,
            });
    }
}

#[derive(Default)]
struct RecordingFieldVisitor {
    fields: BTreeMap<String, String>,
}

impl Visit for RecordingFieldVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.fields
            .insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.fields
            .insert(field.name().to_owned(), value.to_owned());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.fields
            .insert(field.name().to_owned(), value.to_string());
    }
}

fn render_trace_events(events: &[RecordedTracingEvent]) -> String {
    events
        .iter()
        .map(|event| format!("{} {} {:?}", event.target, event.name, event.fields))
        .collect::<Vec<_>>()
        .join("\n")
}

static TRACE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static TRACE_EVENTS: OnceLock<Arc<Mutex<Vec<RecordedTracingEvent>>>> = OnceLock::new();

fn install_test_tracing_recorder() -> Arc<Mutex<Vec<RecordedTracingEvent>>> {
    TRACE_EVENTS
        .get_or_init(|| {
            let layer = RecordingTracingLayer::default();
            let events = layer.events();
            tracing::subscriber::set_global_default(Registry::default().with(layer))
                .expect("install test tracing subscriber");
            tracing::callsite::rebuild_interest_cache();
            events
        })
        .clone()
}

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
struct ApprovalToolProvider {
    calls: AtomicUsize,
}

#[derive(Debug)]
struct StreamingNativeToolProvider {
    calls: AtomicUsize,
}

#[derive(Debug)]
struct FailingProvider;

#[derive(Debug)]
struct MissingToolProvider {
    calls: AtomicUsize,
}

#[derive(Debug)]
struct CancelAfterToolPlanProvider {
    calls: AtomicUsize,
    cancellation: CancellationToken,
}

#[derive(Debug)]
struct MultiToolProvider {
    calls: AtomicUsize,
    tool_names: Vec<&'static str>,
}

#[derive(Debug, Default)]
struct RecordingToolManifestProvider {
    tool_names: Mutex<Vec<String>>,
}

#[derive(Debug, Default)]
struct RecordingAgentLoopHooks {
    calls: Mutex<Vec<RecordedHookCall>>,
}

#[derive(Debug, Clone, serde::Serialize, PartialEq)]
struct RecordedHookCall {
    name: &'static str,
    iteration: u32,
    payload: serde_json::Value,
}

impl RecordingAgentLoopHooks {
    fn calls(&self) -> Vec<RecordedHookCall> {
        self.calls.lock().expect("hook calls").clone()
    }

    fn record(&self, name: &'static str, event: &AgentLoopHookEvent) -> Result<()> {
        self.calls
            .lock()
            .expect("hook calls")
            .push(RecordedHookCall {
                name,
                iteration: event.iteration,
                payload: event.payload.clone(),
            });
        Ok(())
    }
}

mod diagnostics;
mod execution;
mod parsing;
mod persistence;
mod prompt;

fn test_tool_definition(name: &str) -> super::super::AgentLoopToolDefinition {
    super::super::AgentLoopToolDefinition {
        name: name.into(),
        description: format!("{name} test tool"),
        input_schema: json!({"type": "object"}),
        risk: RiskLevel::SafeRead,
        execution_mode: ToolExecutionMode::Parallel,
        timeout_ms: None,
    }
}

impl AgentLoopHooks for RecordingAgentLoopHooks {
    fn before_provider_request(&self, event: &AgentLoopHookEvent) -> Result<()> {
        self.record("before_provider_request", event)
    }

    fn after_provider_response(&self, event: &AgentLoopHookEvent) -> Result<()> {
        self.record("after_provider_response", event)
    }

    fn before_tool_call(&self, event: &AgentLoopHookEvent) -> Result<()> {
        self.record("before_tool_call", event)
    }

    fn after_tool_call(&self, event: &AgentLoopHookEvent) -> Result<()> {
        self.record("after_tool_call", event)
    }
}

#[async_trait]
impl ModelProvider for RecordingToolManifestProvider {
    fn name(&self) -> &str {
        "recording-tools"
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        *self.tool_names.lock().expect("tool names") = request
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<Vec<_>>();
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "recording-tools-model".into(),
            content: r#"{"final_answer":"done"}"#.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            diagnostics: Vec::new(),
        })
    }
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
                diagnostics: Vec::new(),
            });
        }
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "native-model".into(),
            content: r#"{"final_answer":"native done"}"#.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            diagnostics: Vec::new(),
        })
    }
}

#[async_trait]
impl ModelProvider for ApprovalToolProvider {
    fn name(&self) -> &str {
        "approval-native"
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(
            !request.tools.is_empty(),
            "agent loop should expose approval tool definitions"
        );
        if index == 0 {
            return Ok(ModelResponse {
                provider: self.name().into(),
                model: "approval-model".into(),
                content: String::new(),
                tool_calls: vec![ModelToolCall {
                    id: Some("approval-call-1".into()),
                    name: "loop_write".into(),
                    input: json!({"content": "write token=abc123"}),
                    raw_arguments: None,
                }],
                usage: TokenUsage::default(),
                diagnostics: Vec::new(),
            });
        }
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "approval-model".into(),
            content: r#"{"final_answer":"waiting for approval"}"#.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            diagnostics: Vec::new(),
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
            diagnostics: Vec::new(),
        })
    }
}

#[async_trait]
impl ModelProvider for FailingProvider {
    fn name(&self) -> &str {
        "failing"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        Err(IkarosError::Message(
            "agent-loop provider failed token=abc123".into(),
        ))
    }
}

#[async_trait]
impl ModelProvider for CancelAfterToolPlanProvider {
    fn name(&self) -> &str {
        "cancel-after-plan"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.cancellation.cancel();
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "cancel-after-plan-model".into(),
            content: String::new(),
            tool_calls: vec![ModelToolCall {
                id: Some("cancel-call-1".into()),
                name: "loop_echo".into(),
                input: json!({"text": "do not execute token=abc123"}),
                raw_arguments: None,
            }],
            usage: TokenUsage::default(),
            diagnostics: Vec::new(),
        })
    }
}

#[async_trait]
impl ModelProvider for MissingToolProvider {
    fn name(&self) -> &str {
        "missing-tool"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        if index == 0 {
            return Ok(ModelResponse {
                provider: self.name().into(),
                model: "missing-tool-model".into(),
                content: String::new(),
                tool_calls: vec![ModelToolCall {
                    id: Some("missing-call-1".into()),
                    name: "loop_missing".into(),
                    input: json!({"text": "hello token=abc123"}),
                    raw_arguments: None,
                }],
                usage: TokenUsage::default(),
                diagnostics: Vec::new(),
            });
        }
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "missing-tool-model".into(),
            content: r#"{"final_answer":"handled missing tool"}"#.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            diagnostics: Vec::new(),
        })
    }
}

#[async_trait]
impl ModelProvider for MultiToolProvider {
    fn name(&self) -> &str {
        "multi-tool"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        let index = self.calls.fetch_add(1, Ordering::SeqCst);
        if index == 0 {
            let tool_calls = self
                .tool_names
                .iter()
                .enumerate()
                .map(|(index, name)| {
                    json!({
                        "id": format!("call-{index}"),
                        "name": name,
                        "input": {"value": index, "text": "token=abc123"},
                    })
                })
                .collect::<Vec<_>>();
            return Ok(ModelResponse {
                provider: self.name().into(),
                model: "multi-tool-model".into(),
                content: json!({"tool_calls": tool_calls}).to_string(),
                tool_calls: Vec::new(),
                usage: TokenUsage::default(),
                diagnostics: Vec::new(),
            });
        }
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "multi-tool-model".into(),
            content: r#"{"final_answer":"multi done"}"#.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            diagnostics: Vec::new(),
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
                    ..TokenUsage::default()
                },
                events: Vec::new(),
                diagnostics: Vec::new(),
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
                ..TokenUsage::default()
            },
            events: Vec::new(),
            diagnostics: Vec::new(),
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

#[derive(Debug)]
struct WriteSkill;

#[derive(Debug, Default)]
struct ConcurrencyProbe {
    active: AtomicUsize,
    max_active: AtomicUsize,
    calls: AtomicUsize,
}

impl ConcurrencyProbe {
    fn enter(&self) {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        let mut current = self.max_active.load(Ordering::SeqCst);
        while active > current {
            match self.max_active.compare_exchange(
                current,
                active,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => break,
                Err(next) => current = next,
            }
        }
        self.calls.fetch_add(1, Ordering::SeqCst);
    }

    fn exit(&self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
    }

    fn max_active(&self) -> usize {
        self.max_active.load(Ordering::SeqCst)
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[derive(Debug)]
struct ProbeSkill {
    name: &'static str,
    mode: Option<ToolExecutionMode>,
    timeout_ms: Option<u64>,
    delay_ms: u64,
    probe: Arc<ConcurrencyProbe>,
}

#[derive(Debug)]
struct SlowCancellableSkill {
    started: Arc<AtomicUsize>,
    finished: Arc<AtomicUsize>,
}

#[async_trait]
impl Skill for WriteSkill {
    fn name(&self) -> &'static str {
        "loop_write"
    }

    fn description(&self) -> &'static str {
        "writes local state"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::LocalWrite
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        Ok(SkillOutput::new("write ok", json!({"ok": true})))
    }
}

#[async_trait]
impl Skill for ProbeSkill {
    fn name(&self) -> &'static str {
        self.name
    }

    fn description(&self) -> &'static str {
        "records concurrency while simulating tool work"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    fn descriptor(&self) -> SkillDescriptor {
        let mut descriptor = SkillDescriptor::from_skill(self);
        if let Some(mode) = self.mode {
            descriptor.execution_mode = mode;
        }
        descriptor.timeout_ms = self.timeout_ms;
        descriptor
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        self.probe.enter();
        sleep(Duration::from_millis(self.delay_ms)).await;
        self.probe.exit();
        Ok(SkillOutput::new("probe ok", json!({"input": input})))
    }
}

#[async_trait]
impl Skill for SlowCancellableSkill {
    fn name(&self) -> &'static str {
        "slow_cancellable_probe"
    }

    fn description(&self) -> &'static str {
        "simulates in-flight tool work that should be cancellable"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, _input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        self.started.fetch_add(1, Ordering::SeqCst);
        sleep(Duration::from_secs(5)).await;
        self.finished.fetch_add(1, Ordering::SeqCst);
        Ok(SkillOutput::new("slow done", json!({"done": true})))
    }
}

fn write_offline_mock_config(paths: &IkarosPaths) {
    std::fs::create_dir_all(&paths.home).expect("home");
    std::fs::write(
        &paths.config,
        r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
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
