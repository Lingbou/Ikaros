// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[derive(Debug)]
struct DiagnosticProvider {
    diagnostics: Vec<ModelRequestDiagnostic>,
}

#[async_trait]
impl ModelProvider for DiagnosticProvider {
    fn name(&self) -> &str {
        "diagnostic"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        Ok(ModelResponse {
            provider: self.name().into(),
            model: "diagnostic-model".into(),
            content: r#"{"final_answer":"diagnostic-done"}"#.into(),
            tool_calls: Vec::new(),
            usage: TokenUsage::default(),
            diagnostics: self.diagnostics.clone(),
        })
    }
}

#[tokio::test]
async fn agent_loop_emits_model_diagnostic_events_for_provider_diagnostics() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let mut registry = SkillRegistry::new();
    registry.register(EchoSkill {
        calls: Arc::new(AtomicUsize::new(0)),
    });
    let provider = DiagnosticProvider {
        diagnostics: vec![
            ModelRequestDiagnostic {
                kind: "provider_retry_failed".into(),
                message: "provider openai-compatible/kimi-k2.6 retry attempt 1 failed with rate_limit error".into(),
                parameter: None,
            },
            ModelRequestDiagnostic {
                kind: "fallback_provider_selected".into(),
                message: "provider openai-compatible/qwen-2.5-72b selected after 1 fallback attempt(s)".into(),
                parameter: None,
            },
        ],
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("diag-session".into()),
            turn_id: Some("diag-turn".into()),
            task_id: Some("diag-task".into()),
            system_prompt: "Use tools when useful.".into(),
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
    let diagnostic_events: Vec<_> = report
        .events
        .iter()
        .filter_map(|event| match &event.kind {
            AgentEventKind::ModelDiagnostic(diag) => Some(diag),
            _ => None,
        })
        .collect();
    assert_eq!(diagnostic_events.len(), 2);
    assert_eq!(diagnostic_events[0].kind, "provider_retry_failed");
    assert_eq!(diagnostic_events[1].kind, "fallback_provider_selected");
    // Source should be Model (from provider side).
    for event in &report.events {
        if matches!(event.kind, AgentEventKind::ModelDiagnostic(_)) {
            assert_eq!(event.source, AgentEventSource::Model);
        }
    }
}

#[tokio::test]
async fn agent_loop_sanitizes_untrusted_provider_diagnostics_before_events() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let registry = SkillRegistry::new();
    let provider = DiagnosticProvider {
        diagnostics: vec![ModelRequestDiagnostic {
            kind: "provider_retry_failed".into(),
            message: format!(
                "provider leaked sk-secret-value while reporting {}",
                "context ".repeat(200)
            ),
            parameter: Some(format!("api_key={}", "x".repeat(256))),
        }],
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("diag-redaction-session".into()),
            turn_id: Some("diag-redaction-turn".into()),
            task_id: Some("diag-redaction-task".into()),
            system_prompt: "Answer directly.".into(),
            user_input: "start".into(),
        },
        &provider,
        &session,
        &registry,
        AgentLoopOptions::default(),
    )
    .await
    .expect("agent loop");

    let diagnostic = report
        .events
        .iter()
        .find_map(|event| match &event.kind {
            AgentEventKind::ModelDiagnostic(diagnostic) => Some(diagnostic),
            _ => None,
        })
        .expect("diagnostic event");

    let raw = serde_json::to_string(diagnostic).expect("diagnostic json");
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

#[tokio::test(flavor = "current_thread")]
async fn agent_loop_traces_model_result_with_correlation_id_without_prompt_leakage() {
    let _trace_guard = TRACE_TEST_LOCK.lock().await;
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let registry = SkillRegistry::new();
    let provider = DiagnosticProvider {
        diagnostics: vec![ModelRequestDiagnostic {
            kind: "fallback_provider_selected".into(),
            message: "fallback provider openai-compatible/qwen selected".into(),
            parameter: None,
        }],
    };
    let events = install_test_tracing_recorder();
    events.lock().expect("trace events").clear();

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("trace-session".into()),
            turn_id: Some("trace-turn".into()),
            task_id: Some("trace-task".into()),
            system_prompt: "Answer directly.".into(),
            user_input: "hello sk-secret-value".into(),
        },
        &provider,
        &session,
        &registry,
        AgentLoopOptions::default(),
    )
    .await
    .expect("agent loop");

    assert_eq!(report.stop_reason, AgentLoopStopReason::FinalAnswer);
    let events = events.lock().expect("trace events").clone();
    let rendered = render_trace_events(&events);
    assert!(rendered.contains("agent_loop_model_result"));
    assert!(rendered.contains("trace-session"));
    assert!(rendered.contains("trace-turn"));
    assert!(rendered.contains("session:trace-session:turn:trace-turn"));
    assert!(rendered.contains("fallback_provider_selected"));
    assert!(!rendered.contains("sk-secret-value"));
}
