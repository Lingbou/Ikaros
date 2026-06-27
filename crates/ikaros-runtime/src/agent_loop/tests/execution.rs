// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn agent_loop_filters_provider_tool_schema_by_toolset_selection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = SkillRegistry::new();
    registry.register_with_toolset(
        EchoSkill {
            calls: calls.clone(),
        },
        Toolset::Core,
    );
    registry.register_with_toolset(
        ProbeSkill {
            name: "rag_probe",
            mode: None,
            timeout_ms: None,
            delay_ms: 0,
            probe: Arc::new(ConcurrencyProbe::default()),
        },
        Toolset::Rag,
    );
    let provider = RecordingToolManifestProvider::default();

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("toolset-session".into()),
            turn_id: None,
            task_id: Some("toolset-session".into()),
            system_prompt: "Use tools when useful.".into(),
            user_input: "start".into(),
        },
        &provider,
        &session,
        &registry,
        AgentLoopOptions {
            toolsets: ToolsetSelection::new([Toolset::Core]),
            ..AgentLoopOptions::default()
        },
    )
    .await
    .expect("agent loop");

    assert_eq!(report.stop_reason, AgentLoopStopReason::FinalAnswer);
    assert_eq!(
        *provider.tool_names.lock().expect("tool names"),
        vec!["loop_echo".to_string()]
    );
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
            turn_id: None,
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
    let tool_started_event = report
        .events
        .iter()
        .find(|event| {
            matches!(event.kind, AgentEventKind::ToolCallStarted)
                && event
                    .payload
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    == Some("loop_echo")
        })
        .expect("tool started event");
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallOutputDelta))
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallCompleted))
    );
    let events_json = serde_json::to_string(&report.events).expect("events json");
    assert!(!events_json.contains("abc123"));
    assert!(events_json.contains("[REDACTED_SECRET]"));
    let events = session.audit.read_all().expect("audit");
    assert!(events.iter().any(|event| event.kind == "agent_loop_start"));
    assert!(
        events
            .iter()
            .any(|event| event.kind == "agent_loop_model_result")
    );
    let audit_tool_result = events
        .iter()
        .find(|event| event.kind == "tool_result")
        .expect("tool_result audit event");
    assert!(report.events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::AuditAnchor)
            && event
                .payload
                .get("tool_event_id")
                .and_then(serde_json::Value::as_str)
                == Some(tool_started_event.event_id.as_str())
            && event
                .payload
                .get("audit_event_id")
                .and_then(serde_json::Value::as_str)
                == Some(audit_tool_result.id.as_str())
            && event
                .payload
                .get("audit_kind")
                .and_then(serde_json::Value::as_str)
                == Some("tool_result")
    }));
    assert!(events.iter().any(|event| event.kind == "agent_loop_end"));
}

#[tokio::test]
async fn agent_loop_invokes_observer_hooks_with_redacted_payloads() {
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
            r#"{"tool_calls":[{"id":"hook-call-1","name":"loop_echo","input":{"text":"hello token=abc123"}}]}"#.into(),
            r#"{"final_answer":"finished token=abc123"}"#.into(),
        ],
    };
    let hooks = Arc::new(RecordingAgentLoopHooks::default());

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("hook-session".into()),
            turn_id: Some("hook-turn".into()),
            task_id: Some("hook-task".into()),
            system_prompt: "Use tools when useful.".into(),
            user_input: "start token=abc123".into(),
        },
        &provider,
        &session,
        &registry,
        AgentLoopOptions::default().with_hooks(hooks.clone()),
    )
    .await
    .expect("agent loop");

    assert_eq!(report.stop_reason, AgentLoopStopReason::FinalAnswer);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let calls = hooks.calls();
    assert_eq!(
        calls
            .iter()
            .map(|call| (call.name, call.iteration))
            .collect::<Vec<_>>(),
        vec![
            ("before_provider_request", 1),
            ("after_provider_response", 1),
            ("before_tool_call", 1),
            ("after_tool_call", 1),
            ("before_provider_request", 2),
            ("after_provider_response", 2),
        ]
    );
    assert!(calls.iter().any(|call| {
        call.name == "before_tool_call"
            && call.payload.get("name").and_then(serde_json::Value::as_str) == Some("loop_echo")
            && call
                .payload
                .get("tool_event_id")
                .and_then(serde_json::Value::as_str)
                .is_some()
    }));
    assert!(calls.iter().any(|call| {
        call.name == "after_tool_call"
            && call
                .payload
                .get("status")
                .and_then(serde_json::Value::as_str)
                == Some("completed")
    }));
    let rendered = serde_json::to_string(&calls).expect("hook json");
    assert!(!rendered.contains("abc123"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn agent_loop_emits_failed_tool_lifecycle_event() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let registry = SkillRegistry::new();
    let provider = MissingToolProvider {
        calls: AtomicUsize::new(0),
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("missing-tool-session".into()),
            turn_id: Some("missing-tool-turn".into()),
            task_id: Some("missing-tool-task".into()),
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
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallStarted))
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallFailed))
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallCompleted))
    );
    let events_json = serde_json::to_string(&report.events).expect("events json");
    assert!(!events_json.contains("abc123"));
    assert!(events_json.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn agent_loop_runs_parallel_tool_batch_for_parallel_safe_reads() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let probe = Arc::new(ConcurrencyProbe::default());
    let mut registry = SkillRegistry::new();
    registry.register(ProbeSkill {
        name: "parallel_probe_a",
        mode: Some(ToolExecutionMode::Parallel),
        timeout_ms: Some(1_000),
        delay_ms: 50,
        probe: probe.clone(),
    });
    registry.register(ProbeSkill {
        name: "parallel_probe_b",
        mode: Some(ToolExecutionMode::Parallel),
        timeout_ms: Some(1_000),
        delay_ms: 50,
        probe: probe.clone(),
    });
    let provider = MultiToolProvider {
        calls: AtomicUsize::new(0),
        tool_names: vec!["parallel_probe_a", "parallel_probe_b"],
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("parallel-tool-session".into()),
            turn_id: Some("parallel-tool-turn".into()),
            task_id: Some("parallel-tool-task".into()),
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
    assert_eq!(probe.calls(), 2);
    assert_eq!(
        probe.max_active(),
        2,
        "parallel probe tools should overlap in one scheduled batch"
    );
    assert!(report.events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::ToolCallStarted)
            && event
                .payload
                .get("execution_mode")
                .and_then(serde_json::Value::as_str)
                == Some("parallel")
            && event
                .payload
                .get("timeout_ms")
                .and_then(serde_json::Value::as_u64)
                == Some(1_000)
    }));
    let events_json = serde_json::to_string(&report.events).expect("events json");
    assert!(!events_json.contains("abc123"));
}

#[tokio::test]
async fn agent_loop_preserves_model_tool_order_after_parallel_batch() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let probe = Arc::new(ConcurrencyProbe::default());
    let mut registry = SkillRegistry::new();
    registry.register(ProbeSkill {
        name: "parallel_slow_first",
        mode: Some(ToolExecutionMode::Parallel),
        timeout_ms: Some(1_000),
        delay_ms: 90,
        probe: probe.clone(),
    });
    registry.register(ProbeSkill {
        name: "parallel_fast_second",
        mode: Some(ToolExecutionMode::Parallel),
        timeout_ms: Some(1_000),
        delay_ms: 5,
        probe: probe.clone(),
    });
    let provider = MultiToolProvider {
        calls: AtomicUsize::new(0),
        tool_names: vec!["parallel_slow_first", "parallel_fast_second"],
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("parallel-order-session".into()),
            turn_id: Some("parallel-order-turn".into()),
            task_id: Some("parallel-order-task".into()),
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

    assert_eq!(probe.max_active(), 2);
    assert_eq!(
        report
            .tool_results
            .iter()
            .map(|result| result.name.as_str())
            .collect::<Vec<_>>(),
        vec!["parallel_slow_first", "parallel_fast_second"]
    );
}

#[tokio::test]
async fn agent_loop_honors_sequential_tool_execution_mode() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let probe = Arc::new(ConcurrencyProbe::default());
    let mut registry = SkillRegistry::new();
    registry.register(ProbeSkill {
        name: "sequential_probe_a",
        mode: Some(ToolExecutionMode::Sequential),
        timeout_ms: None,
        delay_ms: 20,
        probe: probe.clone(),
    });
    registry.register(ProbeSkill {
        name: "sequential_probe_b",
        mode: Some(ToolExecutionMode::Sequential),
        timeout_ms: None,
        delay_ms: 20,
        probe: probe.clone(),
    });
    let provider = MultiToolProvider {
        calls: AtomicUsize::new(0),
        tool_names: vec!["sequential_probe_a", "sequential_probe_b"],
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("sequential-tool-session".into()),
            turn_id: Some("sequential-tool-turn".into()),
            task_id: Some("sequential-tool-task".into()),
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
    assert_eq!(probe.calls(), 2);
    assert_eq!(
        probe.max_active(),
        1,
        "sequential tool execution mode must not overlap tool calls"
    );
    assert!(report.events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::ToolCallStarted)
            && event
                .payload
                .get("execution_mode")
                .and_then(serde_json::Value::as_str)
                == Some("sequential")
    }));
}

#[tokio::test]
async fn agent_loop_fails_tool_call_on_descriptor_timeout() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let probe = Arc::new(ConcurrencyProbe::default());
    let mut registry = SkillRegistry::new();
    registry.register(ProbeSkill {
        name: "timeout_probe",
        mode: Some(ToolExecutionMode::Sequential),
        timeout_ms: Some(5),
        delay_ms: 100,
        probe,
    });
    let provider = MultiToolProvider {
        calls: AtomicUsize::new(0),
        tool_names: vec!["timeout_probe"],
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("timeout-tool-session".into()),
            turn_id: Some("timeout-tool-turn".into()),
            task_id: Some("timeout-tool-task".into()),
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
    let timeout_event = report
        .events
        .iter()
        .find(|event| {
            matches!(event.kind, AgentEventKind::ToolCallFailed)
                && event
                    .payload
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    == Some("failed")
                && event
                    .payload
                    .get("summary")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|summary| summary.contains("timed out"))
        })
        .expect("timeout tool failure event");
    assert_eq!(
        timeout_event.payload.pointer("/output/timeout/kind"),
        Some(&json!("tool"))
    );
    assert_eq!(
        timeout_event.payload.pointer("/output/timeout/timeout_ms"),
        Some(&json!(5))
    );
    assert_eq!(
        timeout_event.payload.pointer("/output/timeout/reason"),
        Some(&json!("tool_timeout"))
    );
    assert!(
        timeout_event
            .payload
            .pointer("/output/timeout/started_at")
            .and_then(serde_json::Value::as_str)
            .is_some()
    );
    assert!(
        timeout_event
            .payload
            .pointer("/output/timeout/ended_at")
            .and_then(serde_json::Value::as_str)
            .is_some()
    );
    assert_eq!(report.tool_results.len(), 1);
    assert!(report.tool_results[0].recoverable);
    assert_eq!(
        report.tool_results[0].output.pointer("/retry/tool_name"),
        Some(&json!("timeout_probe"))
    );
    assert_eq!(
        report.tool_results[0]
            .output
            .pointer("/retry/tool_input/value"),
        Some(&json!(0))
    );
    let retry_input =
        serde_json::to_string(&report.tool_results[0].output["retry"]).expect("retry input json");
    assert!(!retry_input.contains("abc123"));
    assert!(retry_input.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn agent_loop_cancelled_before_model_request_does_not_call_provider() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let registry = SkillRegistry::new();
    let provider = SequenceProvider {
        calls: AtomicUsize::new(0),
        responses: vec![r#"{"final_answer":"should not run"}"#.into()],
    };
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("cancel-before-model-session".into()),
            turn_id: Some("cancel-before-model-turn".into()),
            task_id: Some("cancel-before-model-task".into()),
            system_prompt: "Use tools when useful.".into(),
            user_input: "start token=abc123".into(),
        },
        &provider,
        &session,
        &registry,
        AgentLoopOptions {
            cancellation,
            ..AgentLoopOptions::default()
        },
    )
    .await
    .expect("agent loop");

    assert_eq!(report.stop_reason, AgentLoopStopReason::Cancelled);
    assert_eq!(report.iterations, 0);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
    assert!(report.events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::TurnEnd)
            && event
                .payload
                .get("stop_reason")
                .and_then(serde_json::Value::as_str)
                == Some("Cancelled")
    }));
    let events_json = serde_json::to_string(&report.events).expect("events json");
    assert!(!events_json.contains("abc123"));
}

#[tokio::test]
async fn agent_loop_cancelled_after_tool_plan_emits_cancelled_tool_event() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = SkillRegistry::new();
    registry.register(EchoSkill {
        calls: calls.clone(),
    });
    let cancellation = CancellationToken::new();
    let provider = CancelAfterToolPlanProvider {
        calls: AtomicUsize::new(0),
        cancellation: cancellation.clone(),
    };

    let report = run_agent_loop(
        AgentLoopInput {
            session_id: Some("cancel-tool-session".into()),
            turn_id: Some("cancel-tool-turn".into()),
            task_id: Some("cancel-tool-task".into()),
            system_prompt: "Use tools when useful.".into(),
            user_input: "start".into(),
        },
        &provider,
        &session,
        &registry,
        AgentLoopOptions {
            cancellation,
            ..AgentLoopOptions::default()
        },
    )
    .await
    .expect("agent loop");

    assert_eq!(report.stop_reason, AgentLoopStopReason::Cancelled);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(report.events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::ToolCallCancelled)
            && event
                .payload
                .get("tool_call_id")
                .and_then(serde_json::Value::as_str)
                == Some("cancel-call-1")
            && event
                .payload
                .get("status")
                .and_then(serde_json::Value::as_str)
                == Some("cancelled")
    }));
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallStarted))
    );
    let events_json = serde_json::to_string(&report.events).expect("events json");
    assert!(!events_json.contains("abc123"));
}

#[tokio::test]
async fn agent_loop_cancels_in_flight_tool_call() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session = ExecutionSession::new(temp.path().join("workspace"), temp.path().join("audit"));
    let started = Arc::new(AtomicUsize::new(0));
    let finished = Arc::new(AtomicUsize::new(0));
    let mut registry = SkillRegistry::new();
    registry.register(SlowCancellableSkill {
        started: started.clone(),
        finished: finished.clone(),
    });
    let provider = MultiToolProvider {
        calls: AtomicUsize::new(0),
        tool_names: vec!["slow_cancellable_probe"],
    };
    let cancellation = CancellationToken::new();
    let canceller = cancellation.clone();
    tokio::spawn(async move {
        sleep(Duration::from_millis(20)).await;
        canceller.cancel();
    });

    let report = tokio::time::timeout(
        Duration::from_secs(1),
        run_agent_loop(
            AgentLoopInput {
                session_id: Some("cancel-in-flight-session".into()),
                turn_id: Some("cancel-in-flight-turn".into()),
                task_id: Some("cancel-in-flight-task".into()),
                system_prompt: "Call the slow tool.".into(),
                user_input: "start slow tool".into(),
            },
            &provider,
            &session,
            &registry,
            AgentLoopOptions {
                cancellation,
                ..AgentLoopOptions::default()
            },
        ),
    )
    .await
    .expect("runtime should return promptly after in-flight cancellation")
    .expect("cancelled report");

    assert_eq!(report.stop_reason, AgentLoopStopReason::Cancelled);
    assert_eq!(started.load(Ordering::SeqCst), 1);
    assert_eq!(finished.load(Ordering::SeqCst), 0);
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallStarted))
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallCancelled))
    );
    assert!(
        !report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallCompleted))
    );
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
            turn_id: None,
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
            turn_id: None,
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
            turn_id: None,
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
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallStarted))
    );
    assert!(
        report
            .events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ToolCallCompleted))
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
