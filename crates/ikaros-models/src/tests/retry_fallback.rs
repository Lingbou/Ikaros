// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn governed_provider_from_config_uses_configured_retry_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = ModelConfig {
        provider: "mock".into(),
        model: "mock-ikaros".into(),
        max_retries: 0,
        ..ModelConfig::default()
    };
    let provider = crate::factory::governed_model_provider_from_config(
        &config,
        &RemoteProviderConfig::default(),
        temp.path(),
        None,
    )
    .expect("governed provider");

    assert_eq!(provider.retry_policy().max_retries, 0);
}

#[test]
fn governed_provider_from_config_builds_configured_fallback_chain() {
    let temp = tempfile::tempdir().expect("tempdir");
    let config = ModelConfig {
        provider: "mock".into(),
        model: "primary-mock".into(),
        fallbacks: vec![ikaros_core::ModelFallbackConfig {
            provider: "mock".into(),
            model: "fallback-mock".into(),
            ..ikaros_core::ModelFallbackConfig::default()
        }],
        ..ModelConfig::default()
    };
    let provider = crate::factory::governed_model_provider_from_config(
        &config,
        &RemoteProviderConfig::default(),
        temp.path(),
        None,
    )
    .expect("governed fallback provider");

    assert_eq!(provider.name(), "fallback-chain");
    assert_eq!(provider.model_id(), "primary-mock");
}

#[tokio::test]
async fn governed_provider_retries_transient_generate_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider = GovernedModelProvider::new_with_retry_policy(
        Box::new(FlakyProvider {
            attempts: attempts.clone(),
            first_error: "provider transient failure: 503 sk-secret".into(),
        }),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
        ProviderRetryPolicy {
            max_retries: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            jitter_ms: 0,
        },
    );

    let response = provider
        .generate(ModelRequest::from_user_text("hello"))
        .await
        .expect("retry succeeds");

    assert_eq!(response.content, "ok");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        response
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["provider_retry_failed", "provider_retry_succeeded"]
    );
    let diagnostics = serde_json::to_string(&response.diagnostics).expect("diagnostics json");
    assert!(diagnostics.contains("transient"));
    assert!(!diagnostics.contains("sk-secret"));
    assert!(diagnostics.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn governed_provider_honors_retry_after_hint_before_backoff() {
    let temp = tempfile::tempdir().expect("tempdir");
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider = GovernedModelProvider::new_with_retry_policy(
        Box::new(FlakyProvider {
            attempts: attempts.clone(),
            first_error: "provider rate limit: 429 Retry-After: 0 sk-secret".into(),
        }),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
        ProviderRetryPolicy {
            max_retries: 1,
            initial_backoff_ms: 100,
            max_backoff_ms: 100,
            jitter_ms: 0,
        },
    );

    let response = tokio::time::timeout(
        std::time::Duration::from_millis(40),
        provider.generate(ModelRequest::from_user_text("hello")),
    )
    .await
    .expect("retry-after zero should avoid configured backoff")
    .expect("retry succeeds");

    assert_eq!(response.content, "ok");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let diagnostics = serde_json::to_string(&response.diagnostics).expect("diagnostics json");
    assert!(diagnostics.contains("retry_after_ms=0"));
    assert!(!diagnostics.contains("sk-secret"));
    assert!(diagnostics.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn governed_provider_retry_diagnostics_include_jitter_when_configured() {
    let temp = tempfile::tempdir().expect("tempdir");
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider = GovernedModelProvider::new_with_retry_policy(
        Box::new(FlakyProvider {
            attempts: attempts.clone(),
            first_error: "provider transient failure: 503 sk-secret".into(),
        }),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
        ProviderRetryPolicy {
            max_retries: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            jitter_ms: 5,
        },
    );

    let response = provider
        .generate(ModelRequest::from_user_text("hello"))
        .await
        .expect("retry succeeds");

    assert_eq!(response.content, "ok");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let diagnostics = serde_json::to_string(&response.diagnostics).expect("diagnostics json");
    assert!(diagnostics.contains("base_retry_delay_ms=0"));
    assert!(diagnostics.contains("jitter_ms="));
    assert!(diagnostics.contains("retry_delay_ms="));
    assert!(!diagnostics.contains("sk-secret"));
    assert!(diagnostics.contains("[REDACTED_SECRET]"));
}

#[tokio::test(flavor = "current_thread")]
async fn governed_provider_emits_structured_retry_traces_without_secret_leakage() {
    let _trace_guard = TRACE_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let attempts = Arc::new(AtomicUsize::new(0));
    let events = install_test_tracing_recorder();
    events.lock().expect("trace events").clear();

    let provider = GovernedModelProvider::new_with_retry_policy(
        Box::new(FlakyProvider {
            attempts: attempts.clone(),
            first_error: format!(
                "provider transient failure: 503 sk-secret {}",
                "x".repeat(80)
            ),
        }),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
        ProviderRetryPolicy {
            max_retries: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            jitter_ms: 0,
        },
    );

    provider
        .generate(ModelRequest::from_user_text("hello"))
        .await
        .expect("retry succeeds");

    let events = events.lock().expect("trace events").clone();
    let rendered = events
        .iter()
        .map(|event| format!("{} {} {:?}", event.target, event.name, event.fields))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("model_request_start"));
    assert!(rendered.contains("provider_retry_failed"));
    assert!(rendered.contains("model_request_complete"));
    assert!(rendered.contains("flaky"));
    assert!(rendered.contains("flaky-model"));
    assert!(rendered.contains("transient"));
    assert!(!rendered.contains("sk-secret"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn governed_provider_does_not_retry_auth_failure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider = GovernedModelProvider::new_with_retry_policy(
        Box::new(FlakyProvider {
            attempts: attempts.clone(),
            first_error: "provider auth failure: 401".into(),
        }),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
        ProviderRetryPolicy {
            max_retries: 3,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            jitter_ms: 0,
        },
    );

    let error = provider
        .generate(ModelRequest::from_user_text("hello"))
        .await
        .expect_err("auth failure is terminal");

    assert!(error.to_string().contains("401"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn governed_provider_stream_reports_retry_diagnostics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let attempts = Arc::new(AtomicUsize::new(0));
    let provider = GovernedModelProvider::new_with_retry_policy(
        Box::new(FlakyProvider {
            attempts: attempts.clone(),
            first_error: "provider transient failure: 503 sk-secret".into(),
        }),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
        ProviderRetryPolicy {
            max_retries: 1,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            jitter_ms: 0,
        },
    );

    let stream = provider
        .stream(ModelRequest::from_user_text("hello"))
        .await
        .expect("retry succeeds");

    assert_eq!(stream.provider, "flaky");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(
        stream
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["provider_retry_failed", "provider_retry_succeeded"]
    );
    let diagnostics = serde_json::to_string(&stream.diagnostics).expect("diagnostics json");
    assert!(diagnostics.contains("transient"));
    assert!(!diagnostics.contains("sk-secret"));
    assert!(diagnostics.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn governed_provider_records_health_failure_and_enforces_cooldown() {
    let temp = tempfile::tempdir().expect("tempdir");
    let provider = GovernedModelProvider::new_with_retry_policy(
        Box::new(AlwaysFailProvider),
        ModelUsageLedger::new(temp.path()),
        ModelRuntimeLimits::default(),
        ProviderRetryPolicy {
            max_retries: 0,
            initial_backoff_ms: 0,
            max_backoff_ms: 0,
            jitter_ms: 0,
        },
    )
    .with_cooldown_policy(ProviderCooldownPolicy {
        failure_threshold: 1,
        cooldown_ms: 60_000,
    });

    let first = provider
        .generate(ModelRequest::from_user_text("hello"))
        .await
        .expect_err("first failure");
    assert!(first.to_string().contains("503"));

    let records = provider.health_ledger().read_all().expect("health");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].provider, "always-fail");
    assert_eq!(records[0].model, "fail-model");
    assert_eq!(records[0].status, ProviderHealthStatus::Unavailable);
    assert_eq!(
        records[0].last_error_kind,
        Some(ProviderErrorKind::Transient)
    );
    assert!(!records[0].last_error_summary.contains("sk-secret"));
    assert!(records[0].cooldown_until.is_some());

    let second = provider
        .generate(ModelRequest::from_user_text("hello again"))
        .await
        .expect_err("cooldown");
    assert!(second.to_string().contains("cooldown"));
}

#[tokio::test]
async fn fallback_provider_uses_next_provider_for_retryable_failure() {
    let seen = Arc::new(Mutex::new(None));
    let provider = FallbackModelProvider::new(vec![
        Box::new(AlwaysFailProvider),
        Box::new(CapturingProvider { seen: seen.clone() }),
    ])
    .expect("fallback chain");

    let response = provider
        .generate(ModelRequest::from_user_text("fallback please"))
        .await
        .expect("fallback succeeds");

    assert_eq!(response.provider, "capturing");
    assert!(
        seen.lock()
            .expect("seen")
            .as_ref()
            .is_some_and(|request| request.messages[0].content == "fallback please")
    );
}

#[tokio::test]
async fn fallback_provider_reports_redacted_failover_diagnostics() {
    let provider = FallbackModelProvider::new(vec![
        Box::new(AlwaysFailProvider),
        Box::new(CapturingProvider {
            seen: Arc::new(Mutex::new(None)),
        }),
    ])
    .expect("fallback chain");

    let response = provider
        .generate(ModelRequest::from_user_text("fallback diagnostics"))
        .await
        .expect("fallback succeeds");

    let kinds = response
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.kind.as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec!["fallback_provider_failed", "fallback_provider_selected"]
    );
    let diagnostics = serde_json::to_string(&response.diagnostics).expect("diagnostics json");
    assert!(diagnostics.contains("always-fail"));
    assert!(diagnostics.contains("capturing"));
    assert!(diagnostics.contains("transient"));
    assert!(!diagnostics.contains("sk-secret"));
    assert!(diagnostics.contains("[REDACTED_SECRET]"));
}

#[tokio::test(flavor = "current_thread")]
async fn fallback_provider_emits_structured_failover_traces_without_secret_leakage() {
    let _trace_guard = TRACE_TEST_LOCK.lock().await;
    let events = install_test_tracing_recorder();
    events.lock().expect("trace events").clear();
    let provider = FallbackModelProvider::new(vec![
        Box::new(AlwaysFailProvider),
        Box::new(CapturingProvider {
            seen: Arc::new(Mutex::new(None)),
        }),
    ])
    .expect("fallback chain");

    provider
        .generate(ModelRequest::from_user_text("fallback diagnostics"))
        .await
        .expect("fallback succeeds");

    let events = events.lock().expect("trace events").clone();
    let rendered = events
        .iter()
        .map(|event| format!("{} {} {:?}", event.target, event.name, event.fields))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(rendered.contains("fallback_provider_failed"));
    assert!(rendered.contains("fallback_provider_selected"));
    assert!(rendered.contains("always-fail"));
    assert!(rendered.contains("capturing"));
    assert!(rendered.contains("transient"));
    assert!(!rendered.contains("sk-secret"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn fallback_provider_stream_reports_failover_diagnostics() {
    let provider = FallbackModelProvider::new(vec![
        Box::new(AlwaysFailProvider),
        Box::new(CapturingProvider {
            seen: Arc::new(Mutex::new(None)),
        }),
    ])
    .expect("fallback chain");

    let stream = provider
        .stream(ModelRequest::from_user_text("fallback stream diagnostics"))
        .await
        .expect("fallback stream succeeds");

    assert_eq!(stream.provider, "capturing");
    assert_eq!(
        stream
            .diagnostics
            .iter()
            .map(|diagnostic| diagnostic.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["fallback_provider_failed", "fallback_provider_selected"]
    );
    let diagnostics = serde_json::to_string(&stream.diagnostics).expect("diagnostics json");
    assert!(!diagnostics.contains("sk-secret"));
    assert!(diagnostics.contains("[REDACTED_SECRET]"));
}
