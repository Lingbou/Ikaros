// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use ikaros_host::{
    WorkbenchModelBudgetStatus, WorkbenchModelCostStatus, WorkbenchProviderFallbackStatus,
    WorkbenchProviderHealthStatus, WorkbenchProviderStatusReport,
};
use tempfile::tempdir;

#[test]
fn model_budget_status_reports_daily_usage_and_remaining_tokens() {
    let budget = WorkbenchModelBudgetStatus {
        daily_token_budget: Some(100),
        used_today: 50,
        remaining_today: Some(50),
        budget_status: "ok",
        suggested_daily_token_budget: 100_000,
    };

    let rendered = format_model_budget_status(&budget);

    assert_eq!(
        rendered,
        "daily_token_budget=100 used_today=50 remaining_today=50 budget_status=ok"
    );
}

#[test]
fn model_budget_status_reports_disabled_daily_budget() {
    let budget = WorkbenchModelBudgetStatus {
        daily_token_budget: None,
        used_today: 0,
        remaining_today: None,
        budget_status: "unbounded",
        suggested_daily_token_budget: 100_000,
    };

    let rendered = format_model_budget_status(&budget);

    assert_eq!(
        rendered,
        "daily_token_budget=disabled used_today=0 remaining_today=unbounded budget_status=unbounded"
    );
}

#[test]
fn model_cost_status_estimates_today_cost_when_pricing_is_known() {
    let cost = WorkbenchModelCostStatus {
        currency: "USD".into(),
        input_per_million: "2.0000".into(),
        output_per_million: "10.0000".into(),
        cache_read_per_million: "0.2000".into(),
        cache_write_per_million: "2.5000".into(),
        estimated_cost_today: "0.000660".into(),
        cache_read_tokens_today: 25,
        cache_write_tokens_today: 10,
        cache_accounting: "priced",
    };

    let rendered = format_model_cost_status(&cost);

    assert_eq!(
        rendered,
        "currency=USD input_per_million=2.0000 output_per_million=10.0000 cache_read_per_million=0.2000 cache_write_per_million=2.5000 estimated_cost_today=0.000660 cache_read_tokens_today=25 cache_write_tokens_today=10 cache_accounting=priced"
    );
}

#[test]
fn model_cost_status_uses_configured_pricing_overlay() {
    let cost = WorkbenchModelCostStatus {
        currency: "CNY".into(),
        input_per_million: "4.0000".into(),
        output_per_million: "16.0000".into(),
        cache_read_per_million: "0.4000".into(),
        cache_write_per_million: "4.0000".into(),
        estimated_cost_today: "0.000222".into(),
        cache_read_tokens_today: 5,
        cache_write_tokens_today: 5,
        cache_accounting: "priced",
    };

    let rendered = format_model_cost_status(&cost);

    assert_eq!(
        rendered,
        "currency=CNY input_per_million=4.0000 output_per_million=16.0000 cache_read_per_million=0.4000 cache_write_per_million=4.0000 estimated_cost_today=0.000222 cache_read_tokens_today=5 cache_write_tokens_today=5 cache_accounting=priced"
    );
}

#[test]
fn model_fallback_status_redacts_endpoint_and_secret_values() {
    let report = sample_provider_status_report();

    let rendered = format_model_fallback_status(&report);

    assert_eq!(
        rendered,
        "fallback_count=1 fallback_chain=0:openai-compatible/fallback-one profile=auto"
    );
    assert!(!rendered.contains("api.example.test"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn screen_provider_cells_summarize_model_cost_fallback_and_debug_without_secrets() {
    let report = sample_provider_status_report();
    let cells = screen_provider_cells(&report);
    let rendered = cells
        .iter()
        .map(WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(cells.len() >= 5);
    assert!(rendered.contains("provider matrix"));
    assert!(rendered.contains("model budget"));
    assert!(rendered.contains("provider recovery"));
    assert!(rendered.contains("profile=local-openai-compatible"));
    assert!(rendered.contains("context_window=131072"));
    assert!(rendered.contains("estimated_cost_today="));
    assert!(rendered.contains("cache_read_tokens_today="));
    assert!(rendered.contains("cache_accounting=tracked"));
    assert!(rendered.contains("provider health"));
    assert!(rendered.contains("health_status=Unknown"));
    assert!(rendered.contains("fallback_count=1"));
    assert!(rendered.contains("/provider matrix --live"));
    assert!(rendered.contains("title=provider cost"));
    assert!(rendered.contains("currency=USD input_per_million=1.0000 output_per_million=2.0000"));
    assert!(rendered.contains("command=/provider debug matrix=/provider matrix"));
    assert!(rendered.contains(
            "matrix=/provider matrix live=/provider matrix --live health=/provider health debug=/provider debug inspect=/provider inspect"
        ));
    assert!(rendered.contains("/provider debug"));
    assert!(!rendered.contains("api.example.test"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn screen_provider_cells_show_health_cooldown_and_error_without_secrets() {
    let mut report = sample_provider_status_report();
    report.health.status = "Unavailable".into();
    report.health.consecutive_failures = 3;
    report.health.last_error_kind = "RateLimited".into();
    report.health.cooldown_until = "2026-06-23T01:01:00Z".into();
    report.health.last_error_summary = "429 rate limited api_key=[REDACTED_SECRET]".into();

    let cells = screen_provider_cells(&report);
    let rendered = cells
        .iter()
        .map(WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("provider health"));
    assert!(rendered.contains("health_status=Unavailable"));
    assert!(rendered.contains("consecutive_failures=3"));
    assert!(rendered.contains("last_error_kind=RateLimited"));
    assert!(rendered.contains("cooldown_until=2026-06-23T01:01:00Z"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("sk-secret-value"));
}

fn sample_provider_status_report() -> WorkbenchProviderStatusReport {
    WorkbenchProviderStatusReport {
        provider: "openai-compatible".into(),
        model: "screen-model".into(),
        configured_profile: "local-openai-compatible".into(),
        provider_profile: "local-openai-compatible".into(),
        profile_source: "explicit",
        context_window: "131072".into(),
        default_output_tokens: "65536".into(),
        tokenizer: "OpenAiCompatible".into(),
        runtime: "harness-agent-loop".into(),
        transport: "openai-compatible-chat-completions".into(),
        temperature_policy: "native".into(),
        reasoning_policy: "native".into(),
        message_policy: "native".into(),
        tool_schema_policy: "native".into(),
        request_body_policy: "native".into(),
        prompt_cache_policy: "native".into(),
        retry_without_parameters: "none".into(),
        streaming: "true".into(),
        tool_calls: "true".into(),
        reasoning: "false".into(),
        json_mode: "true".into(),
        network: "false".into(),
        image_input: "true".into(),
        audio_input: "true".into(),
        file_input: "true".into(),
        health: WorkbenchProviderHealthStatus {
            status: "Unknown".into(),
            consecutive_failures: 0,
            last_error_kind: "none".into(),
            last_error_summary: String::new(),
            cooldown_until: "none".into(),
        },
        budget: WorkbenchModelBudgetStatus {
            daily_token_budget: Some(200),
            used_today: 60,
            remaining_today: Some(140),
            budget_status: "ok",
            suggested_daily_token_budget: 100_000,
        },
        cost: WorkbenchModelCostStatus {
            currency: "USD".into(),
            input_per_million: "1.0000".into(),
            output_per_million: "2.0000".into(),
            cache_read_per_million: "unknown".into(),
            cache_write_per_million: "unknown".into(),
            estimated_cost_today: "0.000080".into(),
            cache_read_tokens_today: 5,
            cache_write_tokens_today: 3,
            cache_accounting: "tracked",
        },
        fallback_count: 1,
        fallback_chain: "0:openai-compatible/fallback-one profile=auto".into(),
        fallbacks: vec![WorkbenchProviderFallbackStatus {
            index: 0,
            provider: "openai-compatible".into(),
            model: "fallback-screen".into(),
            configured_profile: "auto".into(),
            provider_profile: "generic".into(),
            live_smoke: "ready",
            streaming: "true".into(),
            tool_calls: "true".into(),
            reasoning: "false".into(),
            network: "true".into(),
            image_input: "true".into(),
            audio_input: "true".into(),
            file_input: "true".into(),
            context_window: "131072".into(),
            default_output_tokens: "65536".into(),
        }],
    }
}

#[test]
fn screen_gateway_status_cell_reports_worker_lock_without_secret_leakage() {
    let temp = tempdir().expect("tempdir");
    let gateway_dir = temp.path().join("gateway");
    fs::create_dir_all(&gateway_dir).expect("gateway dir");
    fs::write(
        gateway_dir.join("message-worker.lock"),
        "pid=123\nowner=worker-token=abc123\n",
    )
    .expect("worker lock");
    let store = LocalGatewayStore::new(&gateway_dir);
    let message = store
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Task,
            "cancel from screen",
            None,
        ))
        .expect("enqueue");
    store
        .cancel(&message.id, "screen cancel token=abc123")
        .expect("cancel");
    let delivery = store
        .deliver(
            "message-one",
            "chat_response",
            "screen delivery token=abc123",
        )
        .expect("delivery");
    let claim = store
        .claim_pending_deliveries_with_owner(1, "screen-adapter")
        .expect("claim")
        .pop()
        .expect("delivery claim");
    assert_eq!(claim.id, delivery.id);
    store
        .record_delivery_failure_for_claim(&claim, "delivery token=abc123", 2, 30)
        .expect("delivery retry");

    let cell = screen_gateway_status_cell(&store).expect("gateway status cell");
    let rendered = cell.render();

    assert!(rendered.contains("kind=continuation"));
    assert!(rendered.contains("gateway"));
    assert!(rendered.contains("cancelled=1"));
    assert!(rendered.contains("delivery_pending=1"));
    assert!(rendered.contains("delivery_dead_lettered=0"));
    assert!(rendered.contains("lock=present"));
    assert!(rendered.contains("owner=pid=123 owner=[REDACTED_SECRET]"));
    assert!(!rendered.contains("abc123"));
}

#[test]
fn screen_context_cells_summarize_context_diff_compaction_and_references() {
    let session_id = SessionId::from("screen-context-session");
    let turn_id = ikaros_state::session::TurnId::from("screen-context-turn");
    let context_diff = ikaros_state::session::AgentEvent {
        event_id: ikaros_state::session::EventId::from("context-diff"),
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc() - time::Duration::seconds(2),
        source: ikaros_state::session::AgentEventSource::Context,
        kind: AgentEventKind::ContextDiff,
        payload: serde_json::json!({
            "budget": {
                "max_tokens": 4096,
                "used_tokens": 1536,
                "estimator": "heuristic-v1",
                "context_window": 8192,
                "reserved_output_tokens": 1024,
                "source": "provider-window"
            },
            "sections": [
                {
                    "kind": "references",
                    "label": "references",
                    "estimated_tokens": 240,
                    "source_kind": "explicit_reference",
                    "trust_level": "high",
                    "freshness": "current",
                    "scope": "workspace",
                    "injection_reason": "explicit @file",
                    "lines": ["src/lib.rs contains token=sk-secret-value"]
                },
                {
                    "kind": "history",
                    "label": "history",
                    "estimated_tokens": 90,
                    "source_kind": "session_history",
                    "trust_level": "medium",
                    "freshness": "recent",
                    "scope": "session",
                    "injection_reason": "recent turns"
                }
            ],
            "references": [
                {
                    "kind": { "type": "file", "path": "src/lib.rs", "line_range": [1, 12] },
                    "raw": "@file:src/lib.rs:1-12",
                    "resolved_path": "src/lib.rs",
                    "estimated_tokens": 240
                }
            ],
            "prompt_stable_prefix_hash": "fnv1a64:feedface12345678",
            "prompt_stable_prefix_message_count": 1,
            "prompt_stable_prefix_estimated_tokens": 88,
            "compression_summary": "older context compressed token=sk-secret-value",
            "continuation_prompt": "continue from compacted context"
        }),
    };
    let compacted = ikaros_state::session::AgentEvent {
        event_id: ikaros_state::session::EventId::from("context-compacted"),
        session_id: session_id.clone(),
        turn_id,
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc() - time::Duration::seconds(1),
        source: ikaros_state::session::AgentEventSource::Context,
        kind: AgentEventKind::ContextCompacted,
        payload: serde_json::json!({
            "summary": "kept protected reference and removed secret sk-secret-value",
            "compressed_sections": [
                { "kind": "history", "original_tokens": 300, "compressed_tokens": 75 }
            ],
            "continuation_prompt": "continue from compacted context"
        }),
    };
    let replay = SessionReplay {
        session: ikaros_state::session::SessionRecord::new(
            session_id,
            ikaros_state::session::SessionSource::Cli,
        ),
        entries: Vec::new(),
        agent_events: vec![context_diff, compacted],
        approvals: Vec::new(),
    };

    let cells = screen_context_cells_from_replay(&replay).expect("context cells");
    let rendered = cells
        .iter()
        .map(WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("context budget"));
    assert!(rendered.contains("turn=screen-context-turn"));
    assert!(rendered.contains("trace=/trace screen-context-turn"));
    assert!(rendered.contains("estimator=heuristic-v1"), "{rendered}");
    assert!(rendered.contains("used=1536"));
    assert!(rendered.contains("context_window=8192"));
    assert!(rendered.contains("prompt_cache_hash=fnv1a64:feedface12345678"));
    assert!(rendered.contains("prompt_cache_messages=1"));
    assert!(rendered.contains("prompt_cache_estimated=88"));
    assert!(rendered.contains("section references estimated=240"));
    assert!(rendered.contains("trust=high"));
    assert!(rendered.contains("freshness=current"));
    assert!(rendered.contains("scope=workspace"));
    assert!(rendered.contains("section history estimated=90"));
    assert!(rendered.contains("trust=medium"));
    assert!(rendered.contains("freshness=recent"));
    assert!(rendered.contains("scope=session"));
    assert!(rendered.contains("reference @file:src/lib.rs:1-12"));
    assert!(rendered.contains("context compacted"));
    assert!(rendered.contains("compressed_sections=1"));
    assert!(rendered.contains("continuation_prompt=yes"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn screen_failure_cells_surface_latest_failure_recovery_actions_without_secrets() {
    let session_id = SessionId::from("screen-failure-session");
    let turn_id = ikaros_state::session::TurnId::from("screen-failure-turn");
    let event = ikaros_state::session::AgentEvent {
        event_id: ikaros_state::session::EventId::from("screen-failure-event"),
        session_id: session_id.clone(),
        turn_id,
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_state::session::AgentEventSource::Runtime,
        kind: AgentEventKind::Error,
        payload: serde_json::json!({
            "phase": "provider_generate",
            "message": "model daily token budget exceeded: api_key=sk-secret-value",
        }),
    };
    let replay = SessionReplay {
        session: ikaros_state::session::SessionRecord::new(
            session_id,
            ikaros_state::session::SessionSource::Cli,
        ),
        entries: Vec::new(),
        agent_events: vec![event],
        approvals: Vec::new(),
    };

    let cells = screen_failure_cells_from_replay(&replay);
    let rendered = cells
        .iter()
        .map(WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(cells.len(), 1);
    assert!(rendered.contains("latest error"));
    assert!(rendered.contains("kind=budget_exceeded"));
    assert!(rendered.contains("command=/status"));
    assert!(rendered.contains("budget=/budget"));
    assert!(rendered.contains("disable=/budget disable"));
    assert!(rendered.contains("trace=/trace --failed"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn timeline_point_filters_match_failure_and_approval_events() {
    assert!(timeline_point_matches(&AgentEventKind::Error, "failed"));
    assert!(timeline_point_matches(
        &AgentEventKind::ToolCallFailed,
        "failed"
    ));
    assert!(timeline_point_matches(
        &AgentEventKind::ContinuationFailed,
        "failed"
    ));
    assert!(timeline_point_matches(
        &AgentEventKind::ModelDiagnostic(ikaros_providers::model::ModelRequestDiagnostic {
            kind: "provider_retry_failed".into(),
            message: "retry failed for provider".into(),
            parameter: None,
        }),
        "failed"
    ));
    assert!(timeline_point_matches(
        &AgentEventKind::ApprovalRequested,
        "approval"
    ));
    assert!(timeline_point_matches(
        &AgentEventKind::ApprovalResolved,
        "approval"
    ));

    assert!(!timeline_point_matches(&AgentEventKind::TurnEnd, "failed"));
    assert!(!timeline_point_matches(
        &AgentEventKind::ToolCallCompleted,
        "approval"
    ));
}

#[test]
fn screen_timeline_cells_use_recent_session_replay_entries_and_events() {
    let session_id = SessionId::from("screen-session");
    let mut older = ikaros_state::session::SessionEntry::new(
        session_id.clone(),
        ikaros_state::session::SessionEntryKind::UserMessage,
    );
    older.visible_text = Some("old prompt".into());
    older.at = time::OffsetDateTime::now_utc() - time::Duration::minutes(2);
    let mut recent = ikaros_state::session::SessionEntry::new(
        session_id.clone(),
        ikaros_state::session::SessionEntryKind::AssistantMessage,
    );
    recent.visible_text = Some("recent answer".into());
    recent.at = time::OffsetDateTime::now_utc() - time::Duration::minutes(1);
    let event = ikaros_state::session::AgentEvent {
        event_id: ikaros_state::session::EventId::from("event-recent"),
        session_id: session_id.clone(),
        turn_id: ikaros_state::session::TurnId::from("turn-recent"),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: ikaros_state::session::AgentEventSource::Runtime,
        kind: AgentEventKind::TurnEnd,
        payload: serde_json::Value::Null,
    };
    let replay = SessionReplay {
        session: ikaros_state::session::SessionRecord::new(
            session_id,
            ikaros_state::session::SessionSource::Cli,
        ),
        entries: vec![older, recent],
        agent_events: vec![event],
        approvals: Vec::new(),
    };

    let cells = screen_timeline_cells_from_replay(&replay, 2);
    let rendered = cells
        .iter()
        .map(WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(cells.len() >= 2);
    assert!(rendered.contains("recent answer"));
    assert!(rendered.contains("turn_end"));
    assert!(rendered.contains("timeline navigator"));
    assert!(!rendered.contains("old prompt"));
}

#[test]
fn screen_coding_cells_summarize_latest_workflow_groups_with_actions() {
    let session_id = SessionId::from("coding-screen-session");
    let turn_id = ikaros_state::session::TurnId::from("coding-turn");
    let events = [
        ("plan_prepared", "plan ready", time::Duration::minutes(4)),
        (
            "diff_updated",
            "2 files changed token=sk-secret-value",
            time::Duration::minutes(3),
        ),
        (
            "test_evidence_recorded",
            "cargo test failed",
            time::Duration::minutes(2),
        ),
        (
            "review_completed",
            "review found missing assertion",
            time::Duration::minutes(1),
        ),
    ]
    .into_iter()
    .map(|(kind, summary, age)| ikaros_state::session::AgentEvent {
        event_id: ikaros_state::session::EventId::new(),
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc() - age,
        source: ikaros_state::session::AgentEventSource::Tool,
        kind: AgentEventKind::CodingTurn,
        payload: serde_json::json!({
            "kind": kind,
            "summary": summary,
        }),
    })
    .collect::<Vec<_>>();
    let replay = SessionReplay {
        session: ikaros_state::session::SessionRecord::new(
            session_id,
            ikaros_state::session::SessionSource::Cli,
        ),
        entries: Vec::new(),
        agent_events: events,
        approvals: Vec::new(),
    };

    let cells = screen_coding_cells_from_replay(&replay);
    let rendered = cells
        .iter()
        .map(WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(cells.len() >= 4);
    assert!(rendered.contains("coding progress"));
    assert!(rendered.contains("coding diff"));
    assert!(rendered.contains("coding test"));
    assert!(rendered.contains("coding review"));
    assert!(rendered.contains("command=/diff"));
    assert!(rendered.contains("plan=/code plan"));
    assert!(rendered.contains("test=/code test"));
    assert!(rendered.contains("review=/code review"));
    assert!(rendered.contains("rollback=/code rollback"));
    assert!(rendered.contains("turn=coding-turn"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn screen_side_cells_show_continuation_queue_when_no_approvals_are_pending() {
    let queued = test_continuation(
        "queued-continuation",
        ikaros_state::session::SessionContinuationKind::NextTurn,
        ikaros_state::session::SessionContinuationStatus::Queued,
        None,
    );
    let running = test_continuation(
        "running-continuation",
        ikaros_state::session::SessionContinuationKind::ToolResult,
        ikaros_state::session::SessionContinuationStatus::Running,
        Some("worker-one"),
    );

    let cells = screen_side_cells(&[], &[queued, running], &VecDeque::new(), &[]);
    let rendered = cells
        .iter()
        .map(WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("kind=continuation"));
    assert!(rendered.contains("next_turn"));
    assert!(rendered.contains("status=queued"));
    assert!(rendered.contains("tool_result"));
    assert!(rendered.contains("status=running"));
    assert!(rendered.contains("lease_owner=worker-one"));
    assert!(rendered.contains("cancel=/cancel queued-continuation"));
    assert!(rendered.contains("cancel=/cancel running-continuation"));
}

#[test]
fn screen_queue_status_cell_summarizes_continuation_state() {
    let queued = test_continuation(
        "queued-continuation",
        ikaros_state::session::SessionContinuationKind::NextTurn,
        ikaros_state::session::SessionContinuationStatus::Queued,
        None,
    );
    let running = test_continuation(
        "running-continuation",
        ikaros_state::session::SessionContinuationKind::ToolResult,
        ikaros_state::session::SessionContinuationStatus::Running,
        Some("worker-one"),
    );
    let completed = test_continuation(
        "completed-continuation",
        ikaros_state::session::SessionContinuationKind::Retry,
        ikaros_state::session::SessionContinuationStatus::Completed,
        None,
    );

    let rendered = screen_queue_status_cell(&[queued, running, completed]).render();

    assert!(rendered.contains("queued=1"));
    assert!(rendered.contains("running=1"));
    assert!(rendered.contains("completed=1"));
    assert!(rendered.contains("active_kind=tool_result"));
    assert!(rendered.contains("active_id=running-continuation"));
    assert!(rendered.contains("lease_owner=worker-one"));
    assert!(rendered.contains("command=/debug continuations"));
}

#[test]
fn screen_queue_status_cell_redacts_failed_continuation_error() {
    let mut failed = test_continuation(
        "failed-continuation",
        ikaros_state::session::SessionContinuationKind::FollowUp,
        ikaros_state::session::SessionContinuationStatus::Failed,
        None,
    );
    failed.error = Some("provider key sk-secret-value failed".into());

    let rendered = screen_queue_status_cell(&[failed]).render();

    assert!(rendered.contains("failed=1"));
    assert!(rendered.contains("active_kind=follow_up"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn screen_progress_status_cell_renders_latest_progress_without_secret_leakage() {
    let progress = crate::chat::progress::WorkbenchProgressSnapshot {
        kind: "chat_turn".into(),
        status: "failed".into(),
        elapsed_ms: Some(42),
        detail: "provider key [REDACTED_SECRET] failed".into(),
        error_kind: Some("provider_error".into()),
    };

    let rendered = screen_progress_status_cell(Some(&progress), None).render();

    assert!(rendered.contains("cell kind=error title=progress"));
    assert!(rendered.contains("kind=chat_turn"));
    assert!(rendered.contains("status=failed"));
    assert!(rendered.contains("elapsed_ms=42"));
    assert!(rendered.contains("error_kind=provider_error"));
    assert!(rendered.contains("command=/provider debug"));
    assert!(rendered.contains("health=/provider health --live"));
    assert!(rendered.contains("trace=/trace --failed"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn cached_screen_progress_can_be_replaced_for_running_tick() {
    let mut screen = WorkbenchScreen {
        title: "Ikaros".into(),
        status: vec![screen_progress_status_cell(None, None)],
        timeline: Vec::new(),
        main: Vec::new(),
        side: Vec::new(),
        footer: String::new(),
        input_hint: String::new(),
    };
    let progress = crate::chat::progress::WorkbenchProgressSnapshot {
        kind: "chat_turn".into(),
        status: "running".into(),
        elapsed_ms: Some(1_500),
        detail: "hello".into(),
        error_kind: None,
    };

    screen::apply_progress_to_cached_screen(&mut screen, &progress);

    let rendered = screen
        .status
        .iter()
        .find(|cell| cell.title == "progress")
        .expect("progress cell")
        .render();
    assert!(rendered.contains("status=running"));
    assert!(rendered.contains("elapsed_ms=1500"));
    assert!(rendered.contains("detail=hello"));
}

#[test]
fn cached_screen_live_stream_updates_assistant_conversation_cell() {
    let mut screen = WorkbenchScreen {
        title: "Ikaros".into(),
        status: Vec::new(),
        timeline: Vec::new(),
        main: vec![WorkbenchCell {
            kind: super::super::WorkbenchCellKind::Session,
            title: "user turn=turn-one".into(),
            detail: "hello".into(),
        }],
        side: Vec::new(),
        footer: String::new(),
        input_hint: String::new(),
    };
    let session_id = ikaros_state::session::SessionId::from("session-one");
    let turn_id = ikaros_state::session::TurnId::from("turn-one");
    let events = vec![
        ikaros_state::session::AgentEvent::new(
            session_id.clone(),
            turn_id.clone(),
            None,
            ikaros_state::session::AgentEventSource::Model,
            ikaros_state::session::AgentEventKind::ModelStream(
                ikaros_providers::model::ModelStreamEvent::TextDelta("hello, ".into()),
            ),
            serde_json::Value::Null,
        ),
        ikaros_state::session::AgentEvent::new(
            session_id,
            turn_id,
            None,
            ikaros_state::session::AgentEventSource::Model,
            ikaros_state::session::AgentEventKind::ModelStream(
                ikaros_providers::model::ModelStreamEvent::TextDelta("I am Ikaros".into()),
            ),
            serde_json::Value::Null,
        ),
    ];

    screen::apply_live_model_stream_to_cached_screen(&mut screen, &events);

    let assistant = screen
        .main
        .iter()
        .find(|cell| cell.title == "assistant turn=streaming")
        .expect("streaming assistant cell");
    assert_eq!(assistant.kind, super::super::WorkbenchCellKind::Model);
    assert_eq!(assistant.detail, "hello, I am Ikaros");
}

#[test]
fn cached_screen_pending_user_input_is_inserted_after_previous_turn() {
    let mut screen = WorkbenchScreen {
        title: "Ikaros".into(),
        status: Vec::new(),
        timeline: Vec::new(),
        main: vec![
            WorkbenchCell {
                kind: super::super::WorkbenchCellKind::Session,
                title: "user turn=turn-one".into(),
                detail: "first question".into(),
            },
            WorkbenchCell {
                kind: super::super::WorkbenchCellKind::Model,
                title: "assistant turn=turn-one".into(),
                detail: "first answer".into(),
            },
        ],
        side: Vec::new(),
        footer: String::new(),
        input_hint: String::new(),
    };

    screen::apply_pending_user_input_to_cached_screen(&mut screen, "second question");

    let pending = screen.main.last().expect("pending user cell");
    assert_eq!(pending.kind, super::super::WorkbenchCellKind::Session);
    assert_eq!(pending.title, "user turn=pending");
    assert_eq!(pending.detail, "second question");
}

#[test]
fn screen_progress_status_cell_surfaces_approval_pending_actions() {
    let progress = crate::chat::progress::WorkbenchProgressSnapshot {
        kind: "chat_turn".into(),
        status: "approval_pending".into(),
        elapsed_ms: Some(17),
        detail: "pending_approvals=1 new_approvals=1".into(),
        error_kind: None,
    };

    let rendered = screen_progress_status_cell(Some(&progress), None).render();

    assert!(rendered.contains("cell kind=approval title=progress"));
    assert!(rendered.contains("status=approval_pending"));
    assert!(rendered.contains("pending_approvals=1"));
    assert!(rendered.contains("command=/approval"));
    assert!(rendered.contains("approve=/screen approve-selected"));
    assert!(rendered.contains("deny=/screen deny-selected"));
    assert!(rendered.contains("trace=/trace --approval"));
}

#[test]
fn screen_side_cells_show_pending_input_queue_with_clear_action() {
    let mut pending_inputs = std::collections::VecDeque::new();
    pending_inputs.push_back("queued follow up token=sk-secret-value".to_owned());

    let rendered = screen_side_cells(&[], &[], &pending_inputs, &[])
        .iter()
        .map(WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("input queue"));
    assert!(rendered.contains("pending_inputs=1"));
    assert!(rendered.contains("index=1"));
    assert!(rendered.contains("clear=/queue remove 1"));
    assert!(rendered.contains("clear_all=/queue clear"));
    assert!(rendered.contains("message=queued follow up"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn screen_approval_cells_include_inline_approve_and_deny_actions() {
    let pending = ApprovalRecord {
        request: ikaros_execution::harness::ApprovalRequest {
            id: "approval-one".into(),
            call: ikaros_core::ToolCall {
                id: "call-one".into(),
                name: "write_file".into(),
                risk: ikaros_core::RiskLevel::LocalWrite,
                input: serde_json::json!({ "api_key": "sk-secret-value" }),
            },
            reason: "needs write confirmation".into(),
            created_at: "2026-06-23T00:00:00Z".into(),
            workspace_root: Some(std::path::PathBuf::from("/tmp/workspace")),
            context: Some(serde_json::json!({
                "source": "workbench",
                "scope": "workspace"
            })),
        },
        status: ApprovalStatus::Pending,
        updated_at: "2026-06-23T00:00:01Z".into(),
        note: None,
        result: None,
    };

    let rendered = screen_approval_cells(&[pending])
        .iter()
        .map(WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("pending approval-one"));
    assert!(rendered.contains("approval_id=approval-one"));
    assert!(rendered.contains("call_id=call-one"));
    assert!(rendered.contains("approve=/approval approve approval-one"));
    assert!(rendered.contains("deny=/approval deny approval-one"));
    assert!(rendered.contains("tool=write_file"));
    assert!(rendered.contains("risk=LocalWrite"));
    assert!(rendered.contains("scope=workspace"));
    assert!(rendered.contains("input_preview="));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn screen_approval_cells_default_scope_to_session_when_unscoped() {
    let pending = ApprovalRecord {
        request: ikaros_execution::harness::ApprovalRequest {
            id: "approval-session".into(),
            call: ikaros_core::ToolCall {
                id: "call-session".into(),
                name: "task_summarize".into(),
                risk: ikaros_core::RiskLevel::SafeRead,
                input: serde_json::json!({ "text": "summarize this" }),
            },
            reason: "local summary".into(),
            created_at: "2026-06-23T00:00:00Z".into(),
            workspace_root: None,
            context: None,
        },
        status: ApprovalStatus::Pending,
        updated_at: "2026-06-23T00:00:01Z".into(),
        note: None,
        result: None,
    };

    let rendered = screen_approval_cells(&[pending])
        .iter()
        .map(WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("approval_id=approval-session"));
    assert!(rendered.contains("scope=session"));
}

#[test]
fn approval_overlay_json_line_exports_redacted_pending_actions() {
    let pending = ApprovalRecord {
        request: ikaros_execution::harness::ApprovalRequest {
            id: "approval-json".into(),
            call: ikaros_core::ToolCall {
                id: "call-json".into(),
                name: "code_workflow".into(),
                risk: ikaros_core::RiskLevel::LocalWrite,
                input: serde_json::json!({ "api_key": "sk-secret-value" }),
            },
            reason: "approve candidate patch with sk-secret-value".into(),
            created_at: "2026-06-23T00:00:00Z".into(),
            workspace_root: None,
            context: Some(serde_json::json!({
                "operations": {
                    "provider_call": true,
                    "workspace_write": true,
                    "shell": false
                },
                "provider": {"name": "mock"},
                "session": {"session_id": "approval-session", "turn_id": "approval-turn"},
                "patch": {"candidate_diff_chars": 42}
            })),
        },
        status: ApprovalStatus::Pending,
        updated_at: "2026-06-23T00:00:01Z".into(),
        note: None,
        result: None,
    };

    let line = approval_overlay_json_line(std::slice::from_ref(&pending), None);
    let payload = line
        .strip_prefix("approval_overlay_json: ")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .expect("approval overlay JSON payload");

    assert_eq!(payload["schema"], "ikaros-workbench-approval-overlay-v1");
    assert_eq!(payload["version"], 1);
    assert_eq!(payload["pending_count"], 1);
    assert_eq!(payload["items"][0]["id"], "approval-json");
    assert_eq!(payload["items"][0]["tool"], "code_workflow");
    assert_eq!(
        payload["items"][0]["actions"]["approve"],
        "/approval approve approval-json"
    );
    assert_eq!(
        payload["items"][0]["context"]["operations"]["provider_call"],
        true
    );
    assert_eq!(
        payload["items"][0]["context"]["session"]["turn_id"],
        "approval-turn"
    );
    let serialized = serde_json::to_string(&payload).expect("serialize payload");
    assert!(!serialized.contains("sk-secret-value"));
    assert!(serialized.contains("[REDACTED_SECRET]"));
}

fn test_continuation(
    id: &str,
    kind: ikaros_state::session::SessionContinuationKind,
    status: ikaros_state::session::SessionContinuationStatus,
    lease_owner: Option<&str>,
) -> ikaros_state::session::SessionContinuation {
    let now = time::OffsetDateTime::now_utc();
    ikaros_state::session::SessionContinuation {
        continuation_id: ikaros_state::session::ContinuationId::from(id),
        session_id: SessionId::from("screen-session"),
        turn_id: Some(ikaros_state::session::TurnId::from("turn-one")),
        parent_continuation_id: None,
        kind,
        status,
        status_reason: None,
        priority: kind.default_priority(),
        payload: serde_json::Value::Null,
        created_at: now,
        updated_at: now,
        claimed_at: None,
        completed_at: None,
        lease_owner: lease_owner.map(str::to_owned),
        lease_expires_at: None,
        attempt_count: 0,
        error: None,
    }
}
