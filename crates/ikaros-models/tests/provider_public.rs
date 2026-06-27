// SPDX-License-Identifier: GPL-3.0-only

use async_trait::async_trait;
use ikaros_core::{IkarosError, ModelConfig, RemoteProviderConfig, Result};
use ikaros_models::{
    GovernedModelProvider, MockModelProvider, ModelMessage, ModelProvider, ModelRequest,
    ModelRequestDiagnostic, ModelRequestOptions, ModelResponse, ModelRuntimeLimits, ModelStream,
    ModelStreamEvent, ModelStreamEventSink, ModelTokenizerKind, ModelUsageLedger, ModelUsageRecord,
    ProviderErrorKind, ProviderHealthLedger, ProviderHealthRecord, ProviderHealthState,
    ProviderHealthStatus, ProviderRegistry, TokenUsage, governed_provider_from_config,
};
use std::{
    fs,
    io::Write,
    sync::{Arc, Mutex},
};

#[derive(Debug)]
struct CapturingProvider {
    seen: Arc<Mutex<Option<ModelRequest>>>,
}

struct DelayedStreamEventProvider {
    first_chunk_seen: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release_second_chunk: Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

struct RecordingStreamEventSink {
    events: Arc<Mutex<Vec<ModelStreamEvent>>>,
}

#[async_trait]
impl ModelProvider for CapturingProvider {
    fn name(&self) -> &str {
        "capturing"
    }

    async fn generate(&self, request: ModelRequest) -> Result<ModelResponse> {
        *self
            .seen
            .lock()
            .map_err(|_| IkarosError::Message("capture lock poisoned".into()))? = Some(request);
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "capture".into(),
            content: "ok".into(),
            tool_calls: Vec::new(),
            usage: TokenUsage {
                prompt_tokens: Some(1),
                completion_tokens: Some(1),
                total_tokens: None,
                ..TokenUsage::default()
            },
            diagnostics: Vec::new(),
        })
    }
}

#[async_trait]
impl ModelProvider for DelayedStreamEventProvider {
    fn name(&self) -> &str {
        "delayed-stream"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "delayed-model".into(),
            content: "Hello world".into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            diagnostics: Vec::new(),
        })
    }

    async fn stream_with_events(
        &self,
        _request: ModelRequest,
        event_sink: &mut dyn ModelStreamEventSink,
    ) -> Result<ModelStream> {
        event_sink.emit(ModelStreamEvent::Start {
            provider: self.name().into(),
            model: "delayed-model".into(),
        })?;
        event_sink.emit(ModelStreamEvent::TextDelta("Hello ".into()))?;
        if let Some(sender) = self.first_chunk_seen.lock().expect("first sender").take() {
            let _ = sender.send(());
        }
        let receiver = self
            .release_second_chunk
            .lock()
            .expect("release receiver")
            .take();
        if let Some(receiver) = receiver {
            let _ = receiver.await;
        }
        event_sink.emit(ModelStreamEvent::TextDelta("world".into()))?;
        event_sink.emit(ModelStreamEvent::Usage(TokenUsage {
            prompt_tokens: Some(1),
            completion_tokens: Some(2),
            total_tokens: Some(3),
            ..TokenUsage::default()
        }))?;
        event_sink.emit(ModelStreamEvent::Done)?;
        Ok(ModelStream {
            provider: self.name().into(),
            model: "delayed-model".into(),
            chunks: vec!["Hello ".into(), "world".into()],
            tool_calls: Vec::new(),
            usage: TokenUsage {
                prompt_tokens: Some(1),
                completion_tokens: Some(2),
                total_tokens: Some(3),
                ..TokenUsage::default()
            },
            events: vec![
                ModelStreamEvent::Start {
                    provider: self.name().into(),
                    model: "delayed-model".into(),
                },
                ModelStreamEvent::TextDelta("Hello ".into()),
                ModelStreamEvent::TextDelta("world".into()),
                ModelStreamEvent::Usage(TokenUsage {
                    prompt_tokens: Some(1),
                    completion_tokens: Some(2),
                    total_tokens: Some(3),
                    ..TokenUsage::default()
                }),
                ModelStreamEvent::Done,
            ],
            diagnostics: Vec::new(),
        })
    }
}

impl ModelStreamEventSink for RecordingStreamEventSink {
    fn emit(&mut self, event: ModelStreamEvent) -> Result<()> {
        self.events
            .lock()
            .map_err(|_| IkarosError::Message("stream event lock poisoned".into()))?
            .push(event);
        Ok(())
    }
}

#[test]
fn provider_registry_reports_capabilities_cost_and_context() {
    let registry = ProviderRegistry;
    let kimi = registry
        .descriptor(
            "openai-compatible",
            "https://api.moonshot.cn/v1",
            "kimi-k2.6",
        )
        .expect("descriptor");

    assert_eq!(kimi.provider, "openai-compatible");
    assert_eq!(kimi.profile, "moonshot-kimi");
    assert!(kimi.capabilities.chat);
    assert!(kimi.capabilities.tool_calls);
    assert!(kimi.capabilities.streaming);
    assert!(kimi.capabilities.reasoning);
    assert!(kimi.capabilities.network);
    assert_eq!(kimi.context.context_window, 128_000);
    assert_eq!(kimi.context.default_output_tokens, 32_000);
    assert_eq!(kimi.profile_policy.temperature, "omit");
    assert_eq!(kimi.profile_policy.reasoning, "moonshot-kimi");
    assert_eq!(kimi.profile_policy.message, "plain");
    assert_eq!(kimi.profile_policy.tool_schema, "moonshot-subset");
    assert_eq!(kimi.profile_policy.request_body, "none");
    assert_eq!(kimi.profile_policy.prompt_cache, "none");
    assert!(kimi.profile_policy.retry_without_parameters.is_empty());
    assert_eq!(kimi.cost.currency, "USD");
    assert_eq!(kimi.health.status, ProviderHealthStatus::Unknown);
}

#[test]
fn provider_registry_reports_prompt_cache_policy() {
    let registry = ProviderRegistry;
    let anthropic = registry
        .descriptor(
            "anthropic",
            "https://api.anthropic.com/v1",
            "claude-sonnet-4-6",
        )
        .expect("anthropic descriptor");
    let qwen = registry
        .descriptor(
            "openai-compatible",
            "https://dashscope.aliyuncs.com/compatible-mode/v1",
            "qwen3-coder",
        )
        .expect("qwen descriptor");

    assert_eq!(
        anthropic.profile_policy.prompt_cache,
        "anthropic-system-prefix-ephemeral"
    );
    assert_eq!(qwen.profile_policy.prompt_cache, "qwen-system-ephemeral");
}

#[test]
fn provider_registry_honors_configured_openai_compatible_profile() {
    let registry = ProviderRegistry;
    let local = registry
        .descriptor_with_profile(
            "openai-compatible",
            "https://example.invalid/v1",
            "generic-chat",
            "local-openai-compatible",
        )
        .expect("descriptor");

    assert_eq!(local.provider, "openai-compatible");
    assert_eq!(local.profile, "local-openai-compatible");
    assert!(!local.capabilities.network);
    assert!(!local.capabilities.reasoning);
    assert_eq!(local.context.context_window, 131_072);
    assert_eq!(local.context.default_output_tokens, 65_536);
    assert_eq!(local.profile_policy.temperature, "pass-through");
    assert_eq!(local.profile_policy.reasoning, "none");
    assert_eq!(local.profile_policy.message, "plain");
    assert_eq!(local.profile_policy.tool_schema, "pass-through");
    assert_eq!(local.profile_policy.request_body, "none");
    assert_eq!(
        local.profile_policy.retry_without_parameters,
        vec!["temperature".to_owned(), "max_tokens".to_owned()]
    );
}

#[test]
fn provider_registry_keeps_mock_offline_and_free() {
    let registry = ProviderRegistry;
    let mock = registry
        .descriptor("mock", "", "mock-model")
        .expect("mock descriptor");

    assert_eq!(mock.provider, "mock");
    assert!(!mock.capabilities.network);
    assert!(mock.capabilities.streaming);
    assert_eq!(mock.cost.input_per_million, None);
    assert_eq!(mock.cost.output_per_million, None);
    assert_eq!(mock.context.context_window, 8_192);
    assert_eq!(mock.context.tokenizer, ModelTokenizerKind::Mock);
}

#[test]
fn provider_health_records_failure_and_recovery() {
    let mut health = ProviderHealthState::new("openai-compatible", "kimi-k2.6");
    health.record_failure(ProviderErrorKind::RateLimited, "429 rate limited sk-secret");

    assert_eq!(health.status, ProviderHealthStatus::Degraded);
    assert_eq!(health.consecutive_failures, 1);
    assert_eq!(health.last_error_kind, Some(ProviderErrorKind::RateLimited));
    assert!(health.last_error_summary.contains("rate limited"));
    assert!(!health.last_error_summary.contains("sk-secret"));

    health.record_success();

    assert_eq!(health.status, ProviderHealthStatus::Healthy);
    assert_eq!(health.consecutive_failures, 0);
    assert_eq!(health.last_error_kind, None);
    assert!(health.last_error_summary.is_empty());
}

#[test]
fn model_request_diagnostic_redacts_and_caps_fields() {
    let message = format!(
        "provider failed with sk-secret-value and {}",
        "context ".repeat(200)
    );
    let parameter = format!("api_key={}", "x".repeat(256));

    let diagnostic = ModelRequestDiagnostic::new("provider_retry_failed", message, Some(parameter));

    let raw = serde_json::to_string(&diagnostic).expect("diagnostic json");
    assert!(!raw.contains("sk-secret-value"));
    assert!(!raw.contains("api_key=xxx"));
    assert!(raw.contains("[REDACTED_SECRET]"));
    assert!(diagnostic.message.chars().count() <= 512);
    assert!(diagnostic.message.ends_with("...[truncated]"));
    assert!(
        diagnostic
            .parameter
            .as_ref()
            .is_some_and(|parameter| parameter.chars().count() <= 128)
    );
}

#[test]
fn provider_error_classification_is_stable() {
    assert_eq!(
        ProviderErrorKind::classify_status(429),
        ProviderErrorKind::RateLimited
    );
    assert_eq!(
        ProviderErrorKind::classify_status(503),
        ProviderErrorKind::Transient
    );
    assert_eq!(
        ProviderErrorKind::classify_status(401),
        ProviderErrorKind::Auth
    );
    assert_eq!(
        ProviderErrorKind::classify_status(400),
        ProviderErrorKind::BadRequest
    );
    assert!(ProviderErrorKind::RateLimited.retryable());
    assert!(ProviderErrorKind::Transient.retryable());
    assert!(!ProviderErrorKind::Auth.retryable());
    assert!(!ProviderErrorKind::BadRequest.retryable());
}

#[test]
fn governed_provider_delegates_context_profile() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = GovernedModelProvider::new(
        Box::new(MockModelProvider::default()),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
    );

    assert_eq!(
        provider.context_profile().tokenizer,
        ModelTokenizerKind::Mock
    );
    assert_eq!(provider.context_profile().context_window, 8_192);
}

#[tokio::test]
async fn governed_provider_logs_usage_without_prompt_content() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = GovernedModelProvider::new(
        Box::new(MockModelProvider::default()),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
    );
    provider
        .generate(ModelRequest::from_user_text("remember token=abc123"))
        .await
        .expect("generate");
    let raw = fs::read_to_string(provider.ledger().path()).expect("usage log");
    assert!(!raw.contains("remember"));
    assert!(!raw.contains("abc123"));
    let records = provider.ledger().read_all().expect("records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider, "mock");
    assert!(records[0].total_tokens > 0);
}

#[tokio::test]
async fn governed_provider_stream_logs_usage_without_prompt_content() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = GovernedModelProvider::new(
        Box::new(MockModelProvider::default()),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
    );
    let stream = provider
        .stream(ModelRequest::from_user_text("stream token=abc123"))
        .await
        .expect("stream");
    assert!(stream.content().contains("[REDACTED_SECRET]"));
    let raw = fs::read_to_string(provider.ledger().path()).expect("usage log");
    assert!(!raw.contains("stream"));
    assert!(!raw.contains("abc123"));
    let records = provider.ledger().read_all().expect("records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider, "mock");
    assert!(records[0].total_tokens > 0);
}

#[tokio::test]
async fn governed_provider_stream_with_events_preserves_incremental_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let events = Arc::new(Mutex::new(Vec::new()));
    let (first_chunk_seen_tx, first_chunk_seen_rx) = tokio::sync::oneshot::channel();
    let (release_second_chunk_tx, release_second_chunk_rx) = tokio::sync::oneshot::channel();
    let provider = GovernedModelProvider::new(
        Box::new(DelayedStreamEventProvider {
            first_chunk_seen: Mutex::new(Some(first_chunk_seen_tx)),
            release_second_chunk: Mutex::new(Some(release_second_chunk_rx)),
        }),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
    );
    let mut sink = RecordingStreamEventSink {
        events: events.clone(),
    };
    let task = tokio::spawn(async move {
        provider
            .stream_with_events(ModelRequest::from_user_text("stream please"), &mut sink)
            .await
    });

    first_chunk_seen_rx.await.expect("first chunk");
    tokio::task::yield_now().await;
    let live_events = events.lock().expect("events").clone();
    assert!(
        live_events
            .iter()
            .any(|event| matches!(event, ModelStreamEvent::TextDelta(text) if text == "Hello ")),
        "governed provider must not buffer inner stream events: {live_events:?}"
    );
    assert!(!task.is_finished());

    release_second_chunk_tx.send(()).expect("release");
    let stream = task.await.expect("join").expect("stream");
    assert_eq!(stream.content(), "Hello world");
    assert!(matches!(
        events.lock().expect("events").last(),
        Some(ModelStreamEvent::Done)
    ));
}

#[tokio::test]
async fn governed_provider_redacts_request_before_inner_provider() {
    let temp = tempfile::tempdir().expect("tempdir");
    let seen = Arc::new(Mutex::new(None));
    let provider = GovernedModelProvider::new(
        Box::new(CapturingProvider { seen: seen.clone() }),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
    );
    provider
        .generate(ModelRequest::from_user_text("never forward token=abc123"))
        .await
        .expect("generate");
    let captured = seen
        .lock()
        .expect("capture lock")
        .clone()
        .expect("captured request");
    let raw = serde_json::to_string(&captured).expect("json");
    assert!(!raw.contains("abc123"));
    assert!(raw.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn governed_provider_enforces_rate_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = GovernedModelProvider::new(
        Box::new(MockModelProvider::default()),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits {
            rate_limit_per_minute: Some(1),
            daily_token_budget: None,
        },
    );
    provider
        .generate(ModelRequest::from_user_text("first"))
        .await
        .expect("first");
    let err = provider
        .generate(ModelRequest::from_user_text("second"))
        .await
        .expect_err("rate limited");
    assert!(err.to_string().contains("rate limit"));
}

#[tokio::test]
async fn governed_provider_enforces_daily_budget() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = GovernedModelProvider::new(
        Box::new(MockModelProvider::default()),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits {
            rate_limit_per_minute: None,
            daily_token_budget: Some(5),
        },
    );
    let err = provider
        .generate(ModelRequest {
            messages: vec![ModelMessage::user("this request should exceed budget")],
            options: ModelRequestOptions {
                max_tokens: Some(128),
                ..ModelRequestOptions::default()
            },
            tools: Vec::new(),
        })
        .await
        .expect_err("budget limited");
    let err = err.to_string();
    assert!(err.contains("budget"));
    assert!(err.contains("model.default.daily_token_budget"));
}

#[tokio::test]
async fn governed_provider_counts_openai_profile_default_max_tokens() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = ModelConfig {
        provider: "openai-compatible".into(),
        model: "kimi-k2.6".into(),
        daily_token_budget: Some(100),
        ..ModelConfig::default()
    };
    let provider = governed_provider_from_config(
        &config,
        &RemoteProviderConfig {
            api_key: "test-key".into(),
            base_url: "https://api.moonshot.cn/v1".into(),
        },
        temp.path(),
    )
    .expect("governed provider");

    let err = provider
        .generate(ModelRequest::from_user_text("short"))
        .await
        .expect_err("profile default output tokens should exceed budget");
    assert!(err.to_string().contains("budget"));
    assert!(
        err.to_string().contains("3200"),
        "error should include a profile default max-token estimate: {err}"
    );
}

#[test]
fn token_usage_parses_openai_cache_accounting_fields() {
    let openai_usage: TokenUsage = serde_json::from_value(serde_json::json!({
        "prompt_tokens": 100,
        "completion_tokens": 20,
        "total_tokens": 120,
        "prompt_tokens_details": {
            "cached_tokens": 64
        }
    }))
    .expect("openai usage");
    assert_eq!(openai_usage.cache_read_tokens, Some(64));
    assert_eq!(openai_usage.cache_write_tokens, None);
}

#[test]
fn model_usage_ledger_round_trips_cache_accounting_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ledger = ModelUsageLedger::from_file(temp.path().join("usage.jsonl"));

    ledger
        .append(ModelUsageRecord {
            id: "usage-cache".into(),
            at: "2026-06-22T00:00:00Z".into(),
            provider: "anthropic".into(),
            model: "claude-sonnet-4-5".into(),
            prompt_tokens: Some(10),
            completion_tokens: Some(4),
            total_tokens: 14,
            cache_read_tokens: Some(9),
            cache_write_tokens: Some(7),
            estimated: false,
        })
        .expect("append usage");

    let records = ledger.read_all().expect("usage records");

    assert_eq!(records[0].cache_read_tokens, Some(9));
    assert_eq!(records[0].cache_write_tokens, Some(7));
}

#[test]
fn model_usage_ledger_keeps_cached_totals_when_log_has_partial_trailing_line() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("usage.jsonl");
    let ledger = ModelUsageLedger::from_file(&path);

    ledger
        .append(ModelUsageRecord {
            id: "usage-cache-total".into(),
            at: "2026-06-22T12:00:00Z".into(),
            provider: "mock".into(),
            model: "mock-model".into(),
            prompt_tokens: Some(6),
            completion_tokens: Some(4),
            total_tokens: 10,
            cache_read_tokens: None,
            cache_write_tokens: None,
            estimated: false,
        })
        .expect("append usage");
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open usage log")
        .write_all(br#"{"id":"partial""#)
        .expect("write partial line");

    assert_eq!(
        ledger
            .total_for_day("2026-06-22")
            .expect("cached daily total"),
        10
    );
}

#[test]
fn model_usage_ledger_refreshes_cache_after_external_append() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("usage.jsonl");
    let ledger = ModelUsageLedger::from_file(&path);
    let external = ModelUsageLedger::from_file(&path);

    ledger
        .append(ModelUsageRecord {
            id: "usage-cache-first".into(),
            at: "2026-06-22T12:00:00Z".into(),
            provider: "mock".into(),
            model: "mock-model".into(),
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: 10,
            cache_read_tokens: None,
            cache_write_tokens: None,
            estimated: true,
        })
        .expect("append first usage");
    assert_eq!(ledger.total_for_day("2026-06-22").expect("first total"), 10);

    external
        .append(ModelUsageRecord {
            id: "usage-cache-external".into(),
            at: "2026-06-22T12:05:00Z".into(),
            provider: "mock".into(),
            model: "mock-model".into(),
            prompt_tokens: None,
            completion_tokens: None,
            total_tokens: 7,
            cache_read_tokens: None,
            cache_write_tokens: None,
            estimated: true,
        })
        .expect("append external usage");

    assert_eq!(
        ledger
            .total_for_day("2026-06-22")
            .expect("refreshed daily total"),
        17
    );
}

#[test]
fn provider_health_ledger_keeps_cached_latest_when_log_has_partial_trailing_line() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("provider-health.jsonl");
    let ledger = ProviderHealthLedger::from_file(&path);

    ledger
        .append(ProviderHealthRecord {
            at: "2026-06-22T12:00:00Z".into(),
            provider: "mock".into(),
            model: "mock-model".into(),
            status: ProviderHealthStatus::Healthy,
            consecutive_failures: 0,
            last_error_kind: None,
            last_error_summary: String::new(),
            cooldown_until: None,
        })
        .expect("append health");
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open health log")
        .write_all(br#"{"provider":"partial""#)
        .expect("write partial line");

    let latest = ledger
        .latest("mock", "mock-model")
        .expect("cached health latest")
        .expect("latest health");
    assert_eq!(latest.status, ProviderHealthStatus::Healthy);
}

#[test]
fn provider_health_ledger_refreshes_cache_after_external_append() {
    let temp = tempfile::tempdir().expect("tempdir");
    let path = temp.path().join("provider-health.jsonl");
    let ledger = ProviderHealthLedger::from_file(&path);
    let external = ProviderHealthLedger::from_file(&path);

    ledger
        .append(ProviderHealthRecord {
            at: "2026-06-22T12:00:00Z".into(),
            provider: "mock".into(),
            model: "mock-model".into(),
            status: ProviderHealthStatus::Healthy,
            consecutive_failures: 0,
            last_error_kind: None,
            last_error_summary: String::new(),
            cooldown_until: None,
        })
        .expect("append first health");
    assert_eq!(
        ledger
            .latest("mock", "mock-model")
            .expect("first latest")
            .expect("first health")
            .status,
        ProviderHealthStatus::Healthy
    );

    external
        .append(ProviderHealthRecord {
            at: "2026-06-22T12:05:00Z".into(),
            provider: "mock".into(),
            model: "mock-model".into(),
            status: ProviderHealthStatus::Degraded,
            consecutive_failures: 1,
            last_error_kind: Some(ProviderErrorKind::RateLimited),
            last_error_summary: "retry-after 1s".into(),
            cooldown_until: Some("2026-06-22T12:06:00Z".into()),
        })
        .expect("append external health");

    let latest = ledger
        .latest("mock", "mock-model")
        .expect("refreshed latest")
        .expect("refreshed health");
    assert_eq!(latest.status, ProviderHealthStatus::Degraded);
    assert_eq!(latest.consecutive_failures, 1);
    assert_eq!(latest.last_error_kind, Some(ProviderErrorKind::RateLimited));
}
