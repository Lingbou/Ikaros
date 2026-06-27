// SPDX-License-Identifier: GPL-3.0-only

use super::initial_interactive_runtime;
use super::interactive::{
    InteractiveChatStatusInput, WorkbenchCancelTarget, available_agent_lines,
    cancel_selected_screen_continuation, cancel_session_continuations, clear_interactive_session,
    continuations_json_line, format_interactive_chat_status, should_emit_default_inline_stdout,
};
use super::{
    interactive_chat_turn_error_json_line, interactive_chat_turn_recovery_hint,
    interactive_command_error_json_line,
};
use crate::resolve_agent_instance;
use ikaros_core::{
    AgentAuthScope, AgentInstanceConfig, AgentPermission, IkarosConfig, PolicyDecision,
};
use ikaros_harness::ExecutionSession;
use ikaros_models::{ModelStreamEvent, ModelUsageLedger, TokenUsage};
use ikaros_runtime::ChatRunOptions;
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSink, AgentEventSource, EventId,
    SessionContinuationClaim, SessionContinuationInput, SessionContinuationKind,
    SessionContinuationStatus, SessionEntry, SessionEntryKind, SessionId, SessionRecord,
    SessionSource, SessionStore, SqliteSessionStore, TurnId,
};
use std::{collections::VecDeque, fs};
use time::{Duration, OffsetDateTime};

#[test]
fn interactive_input_sanitizer_drops_mouse_escape_sequences() {
    let input = "\u{1b}[<35;55;37M你好\u{1b}[<35;55;37m";

    assert_eq!(super::sanitize_interactive_input(input), "你好");
}

#[test]
fn default_inline_stdout_policy_follows_default_ui_mode() {
    assert!(should_emit_default_inline_stdout(true, true));
    assert!(should_emit_default_inline_stdout(true, false));
    assert!(!should_emit_default_inline_stdout(false, true));
}

#[test]
fn interactive_chat_lists_and_resolves_agent_profiles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut config = IkarosConfig::default();
    config.agent.instances.insert(
        "repo-build".into(),
        AgentInstanceConfig {
            profile: "build".into(),
            ..AgentInstanceConfig::default()
        },
    );
    let lines = available_agent_lines(&config, "repo-build");
    assert!(lines.iter().any(|line| line.contains("plan mode=plan")));
    assert!(lines.iter().any(|line| line.contains("build mode=build")));
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("* repo-build instance profile=build"))
    );

    let agent = resolve_agent_instance(
        &config,
        Some("repo-build"),
        temp.path(),
        &temp.path().join("home"),
    )
    .expect("repo-build");
    assert_eq!(agent.agent_id, "repo-build");
    assert_eq!(agent.profile_name, "build");
    assert_eq!(agent.profile.mode.as_str(), "build");

    let error = resolve_agent_instance(
        &config,
        Some("missing"),
        temp.path(),
        &temp.path().join("home"),
    )
    .expect_err("missing");
    assert!(error.to_string().contains("missing"));
}

#[test]
fn interactive_chat_status_reports_active_runtime() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let paths = ikaros_core::IkarosPaths::from_home(temp.path().join("home"));
    paths.ensure().expect("paths");
    let agent = IkarosConfig::default().agent.active();
    let session = ExecutionSession::new_with_agent(&workspace, &paths.audit_dir, &agent);
    let usage = ModelUsageLedger::new(&paths.audit_dir);
    let options = ChatRunOptions {
        stream: true,
        scope: Some("repo".into()),
        ..ChatRunOptions::default()
    };

    let status = format_interactive_chat_status(InteractiveChatStatusInput {
        agent: &agent,
        session: &session,
        chat_session_id: "chat-session",
        state_dir: &paths.home,
        options: &options,
        emotion: "Neutral",
        usage_ledger: &usage,
    });
    assert!(status.contains("agent=build"));
    assert!(status.contains("mode=build"));
    assert!(status.contains("emotion=Neutral"));
    assert!(status.contains("stream=true"));
    assert!(status.contains("history_context_limit=3"));
    assert!(status.contains("history_summary_limit=12"));
    assert!(status.contains("context_token_budget=2000"));
    assert!(status.contains("relationship_learning=true"));
    assert!(status.contains("agent_loop=true"));
    assert!(status.contains("scope=repo"));
    assert!(status.contains("chat_session=chat-session"));
    assert!(status.contains("session_state_db="));
    assert!(status.contains("chat_timeline=session_store"));
    assert!(status.contains("audit="));
    assert!(status.contains("model_usage="));
    assert!(!status.contains("chat_history="));
}

#[test]
fn default_interactive_chat_session_resumes_latest_matching_nonempty_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = ikaros_core::IkarosPaths::from_home(temp.path().join("home"));
    paths.ensure().expect("paths");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let config = IkarosConfig::default();
    let agent = resolve_agent_instance(&config, None, &workspace, &paths.home).expect("agent");
    let store = SqliteSessionStore::new(&agent.state_dir);
    let base = OffsetDateTime::UNIX_EPOCH + Duration::days(1);

    append_chat_session_fixture(
        &store,
        "older-session",
        &agent.agent_id,
        &workspace,
        base,
        "older prompt",
    );
    append_empty_session_fixture(
        &store,
        "empty-newer-session",
        &agent.agent_id,
        &workspace,
        base + Duration::seconds(2),
    );
    append_chat_session_fixture(
        &store,
        "newer-session",
        &agent.agent_id,
        &workspace,
        base + Duration::seconds(1),
        "newer prompt",
    );

    let selected = super::interactive_chat_session_id(&config, &paths, &workspace, None, None)
        .expect("selected session");
    assert_eq!(selected, "newer-session");

    let explicit = super::interactive_chat_session_id(
        &config,
        &paths,
        &workspace,
        None,
        Some("manual-session"),
    )
    .expect("explicit session");
    assert_eq!(explicit, "manual-session");
}

fn append_empty_session_fixture(
    store: &SqliteSessionStore,
    session_id: &str,
    agent_id: &str,
    workspace: &std::path::Path,
    started_at: OffsetDateTime,
) {
    let mut session = SessionRecord::new(session_id, SessionSource::Cli);
    session.agent_id = Some(agent_id.to_owned());
    session.workspace = Some(workspace.to_path_buf());
    session.started_at = started_at;
    store.upsert_session(&session).expect("upsert session");
}

fn append_chat_session_fixture(
    store: &SqliteSessionStore,
    session_id: &str,
    agent_id: &str,
    workspace: &std::path::Path,
    started_at: OffsetDateTime,
    user_message: &str,
) {
    append_empty_session_fixture(store, session_id, agent_id, workspace, started_at);
    let turn_id = TurnId::from(format!("{session_id}-turn"));
    let mut user = SessionEntry::new(session_id, SessionEntryKind::UserMessage);
    user.turn_id = Some(turn_id.clone());
    user.at = started_at + Duration::seconds(1);
    user.visible_text = Some(user_message.to_owned());

    let mut assistant = SessionEntry::new(session_id, SessionEntryKind::AssistantMessage);
    assistant.parent_entry_id = Some(user.entry_id.clone());
    assistant.turn_id = Some(turn_id);
    assistant.at = started_at + Duration::seconds(2);
    assistant.visible_text = Some(format!("assistant remembered {user_message}"));
    assistant.payload = serde_json::json!({
        "agent": agent_id,
        "provider": "mock",
        "model": "mock-chat",
        "streamed": false,
    });

    store.append_entry(&user).expect("append user");
    store.append_entry(&assistant).expect("append assistant");
}

#[test]
fn default_inline_model_command_lines_do_not_leak_protocol_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = ikaros_core::IkarosPaths::from_home(temp.path().join("home"));
    paths.ensure().expect("paths");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let mut config = IkarosConfig::default();
    config.model.default.provider = "mock".into();
    config.model.default.runtime = "harness-agent-loop".into();
    config.model.default.transport = "mock".into();
    config.model.default.model = "mock-ikaros".into();
    let (mut runtime, _registry) = initial_interactive_runtime(
        &paths,
        &workspace,
        &config,
        None,
        "inline-model-session".into(),
    )
    .expect("interactive runtime");
    runtime.default_inline_ui = true;

    let lines = super::workbench::model_status_human_lines(&paths, &runtime).expect("model lines");

    assert_default_inline_command_lines_are_human("/model", &lines);
    let rendered = lines.join("\n");
    assert!(rendered.contains("• Model"));
    assert!(!rendered.contains("model_source:"));
}

#[test]
fn default_inline_memory_command_lines_do_not_leak_protocol_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = ikaros_core::IkarosPaths::from_home(temp.path().join("home"));
    paths.ensure().expect("paths");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let mut config = IkarosConfig::default();
    config.model.default.provider = "mock".into();
    config.model.default.runtime = "harness-agent-loop".into();
    config.model.default.transport = "mock".into();
    config.model.default.model = "mock-ikaros".into();
    let (mut runtime, _registry) = initial_interactive_runtime(
        &paths,
        &workspace,
        &config,
        None,
        "inline-memory-session".into(),
    )
    .expect("interactive runtime");
    runtime.default_inline_ui = true;

    let lines = super::workbench::memory_status_human_lines(&config, &paths, &runtime)
        .expect("memory lines");

    assert_default_inline_command_lines_are_human("/memory", &lines);
    let rendered = lines.join("\n");
    assert!(rendered.contains("• Memory"));
    assert!(!rendered.contains("memory_status_json:"));
}

#[test]
fn default_inline_common_inspect_command_lines_do_not_leak_protocol_fields() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = ikaros_core::IkarosPaths::from_home(temp.path().join("home"));
    paths.ensure().expect("paths");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join("src")).expect("workspace src");
    fs::write(workspace.join("src/lib.rs"), "pub fn smoke() {}\n").expect("workspace file");
    let mut config = IkarosConfig::default();
    config.model.default.provider = "mock".into();
    config.model.default.runtime = "harness-agent-loop".into();
    config.model.default.transport = "mock".into();
    config.model.default.model = "mock-ikaros".into();
    let (mut runtime, registry) = initial_interactive_runtime(
        &paths,
        &workspace,
        &config,
        None,
        "inline-common-inspect-session".into(),
    )
    .expect("interactive runtime");
    runtime.default_inline_ui = true;
    let options = ChatRunOptions {
        stream: true,
        ..ChatRunOptions::default()
    };
    let usage = ModelUsageLedger::new(&paths.audit_dir);

    let mut lines = Vec::new();
    lines.extend(
        super::workbench::workbench_status_human_lines(
            &config, &paths, &workspace, &runtime, &options, &usage,
        )
        .expect("status lines"),
    );
    lines.extend(
        super::workbench::context_status_human_lines(&runtime, &options).expect("context lines"),
    );
    lines.extend(super::workbench::rag_status_human_lines(
        &config, &paths, &options,
    ));
    lines.extend(
        super::workbench::tools_status_human_lines(&registry, &runtime.agent).expect("tools lines"),
    );
    lines.extend(super::workbench::mcp_status_human_lines(&config));
    lines.extend(super::workbench::api_status_human_lines(&config));
    lines.extend(super::workbench::tasks_status_human_lines(&paths).expect("tasks lines"));
    lines.extend(super::workbench::gateway_status_human_lines(&paths).expect("gateway lines"));
    lines.extend(
        super::workbench::context_mentions_human_lines(&workspace, Some("lib"))
            .expect("mentions lines"),
    );

    assert_default_inline_command_lines_are_human("common inspect", &lines);
    let rendered = lines.join("\n");
    for title in [
        "• Session",
        "• Status",
        "• Context",
        "• RAG",
        "• Tools",
        "• MCP",
        "• API",
        "• Tasks",
        "• Gateway",
        "• Mentions",
    ] {
        assert!(rendered.contains(title), "missing {title} in:\n{rendered}");
    }
}

fn assert_default_inline_command_lines_are_human(command: &str, lines: &[String]) {
    assert!(!lines.is_empty(), "{command} produced no output");
    let rendered = lines.join("\n");
    for leaked in [
        "workbench_evidence",
        "entry=",
        "Notice:",
        "notice_kind=",
        "_json:",
        "kind=",
        "screen_json:",
        "status_model:",
        "context_session:",
        "memory_status_json:",
        "rag_status_json:",
        "tools_status_json:",
        "mcp_status_json:",
        "api_status_json:",
        "tasks_total:",
        "mentions_query:",
        "Tip: Use /mcp",
        "pending input queue is empty",
        "session_state_db:",
        "audit:",
        "model_usage:",
    ] {
        assert!(
            !rendered.contains(leaked),
            "{command} leaked {leaked} in:\n{rendered}"
        );
    }
}

#[test]
fn interactive_chat_turn_error_json_line_classifies_budget_errors() {
    let error = anyhow::anyhow!(
        "model daily token budget exceeded: used 69903, estimated request 34775, budget 100000; api_key=sk-secret-value"
    );

    let line = interactive_chat_turn_error_json_line("budget-session", &error);

    assert!(line.contains("chat_turn_error_json:"));
    assert!(line.contains("\"schema\":\"ikaros-workbench-chat-turn-error-v1\""));
    assert!(line.contains("\"error_kind\":\"budget_exceeded\""));
    assert!(line.contains("\"session_id\":\"budget-session\""));
    assert!(line.contains("\"status\":\"failed\""));
    assert!(line.contains("\"/status\""));
    assert!(line.contains("\"/budget\""));
    assert!(line.contains("\"/budget set <tokens>\""));
    assert!(line.contains("\"/budget disable\""));
    assert!(line.contains("model.default.daily_token_budget"));
    assert!(line.contains("[REDACTED_SECRET]"));
    assert!(!line.contains("sk-secret-value"));
}

#[test]
fn interactive_chat_turn_recovery_hint_explains_provider_errors() {
    let error = anyhow::anyhow!("provider http timeout after retry");

    let hint = interactive_chat_turn_recovery_hint(&error).expect("provider hint");

    assert!(hint.contains("chat_turn_recovery_hint:"));
    assert!(hint.contains("/provider debug"));
    assert!(hint.contains("/provider health --live"));
}

#[test]
fn interactive_command_error_json_line_is_recoverable_and_redacted() {
    let error = anyhow::anyhow!("network egress denied for api_key=sk-secret-value");

    let line =
        interactive_command_error_json_line("/mcp call-http http://example.test search", &error);

    assert!(line.contains("interactive_command_error_json:"));
    assert!(line.contains("\"schema\":\"ikaros-interactive-command-error-v1\""));
    assert!(line.contains("\"command\":\"/mcp\""));
    assert!(line.contains("\"recoverable\":true"));
    assert!(line.contains("\"error_kind\":\"provider_error\""));
    assert!(line.contains("\"/help\""));
    assert!(line.contains("\"/commands\""));
    assert!(line.contains("\"/mcp status\""));
    assert!(line.contains("\"/provider health --live\""));
    assert!(line.contains("[REDACTED_SECRET]"));
    assert!(!line.contains("sk-secret-value"));
}

#[test]
fn interactive_chat_initial_runtime_resolves_agent_instances() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = ikaros_core::IkarosPaths::from_home(temp.path().join("home"));
    paths.ensure().expect("paths");
    let fallback_workspace = temp.path().join("fallback-workspace");
    let instance_workspace = temp.path().join("instance-workspace");
    fs::create_dir_all(&fallback_workspace).expect("fallback workspace");
    fs::create_dir_all(&instance_workspace).expect("instance workspace");
    let mut config = IkarosConfig::default();
    config.model.default.provider = "mock".into();
    config.model.default.runtime = "harness-agent-loop".into();
    config.model.default.transport = "mock".into();
    config.model.default.model = "mock-ikaros".into();
    config.agent.instances.insert(
        "repo-build".into(),
        AgentInstanceConfig {
            profile: "build".into(),
            workspace: Some(instance_workspace.clone()),
            auth_scope: AgentAuthScope {
                local_only: true,
                allow_network: AgentPermission::Deny,
            },
            ..AgentInstanceConfig::default()
        },
    );

    let (runtime, _registry) = initial_interactive_runtime(
        &paths,
        &fallback_workspace,
        &config,
        Some("repo-build"),
        "chat-session".into(),
    )
    .expect("interactive runtime");

    assert_eq!(runtime.agent.name, "build");
    assert_eq!(runtime.session.sandbox.workspace_root, instance_workspace);
    let overlay = runtime
        .session
        .sandbox
        .agent
        .as_ref()
        .expect("agent overlay");
    assert_eq!(overlay.agent_id.as_deref(), Some("repo-build"));
    assert_eq!(overlay.network, PolicyDecision::Deny);
}

#[test]
fn clear_interactive_session_resets_visible_state_and_updates_options() {
    let temp = tempfile::tempdir().expect("tempdir");
    let paths = ikaros_core::IkarosPaths::from_home(temp.path().join("home"));
    paths.ensure().expect("paths");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let mut config = IkarosConfig::default();
    config.model.default.provider = "mock".into();
    config.model.default.runtime = "harness-agent-loop".into();
    config.model.default.transport = "mock".into();
    config.model.default.model = "mock-ikaros".into();
    let (mut runtime, _registry) = initial_interactive_runtime(
        &paths,
        &workspace,
        &config,
        None,
        "clear-old-session".into(),
    )
    .expect("interactive runtime");
    let mut options = ChatRunOptions {
        session_id: Some("clear-old-session".into()),
        ..ChatRunOptions::default()
    };
    runtime.pending_inputs.push_back("queued input".into());
    super::workbench::apply_workbench_screen_args(
        &mut runtime.screen_state,
        &["--palette-query", "/provider"],
    )
    .expect("screen state");

    let report = clear_interactive_session(&mut runtime, &mut options);

    assert_eq!(report.old_session_id, "clear-old-session");
    assert_ne!(report.new_session_id, report.old_session_id);
    assert_eq!(report.pending_inputs_cleared, 1);
    assert_eq!(runtime.pending_inputs.len(), 0);
    assert_eq!(runtime.chat_session_id, report.new_session_id);
    assert_eq!(
        options.session_id.as_deref(),
        Some(report.new_session_id.as_str())
    );
    assert!(!runtime.screen_state.command_palette_open());
}

#[test]
fn workbench_cancel_marks_queued_and_running_continuations_cancelled() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("cancel-session");
    let queued = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::NextTurn,
        ))
        .expect("queued continuation");
    let running = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::ToolResult,
        ))
        .expect("running continuation");
    let claimed = store
        .claim_next_continuation(
            &SessionContinuationClaim::for_session(session_id.clone())
                .with_lease_owner("test-worker")
                .with_lease_duration_seconds(30),
        )
        .expect("claim")
        .expect("claimed continuation");
    assert_eq!(claimed.continuation_id, queued.continuation_id);

    let report = cancel_session_continuations(
        &store,
        &session_id,
        WorkbenchCancelTarget::All,
        "operator requested cancel",
    )
    .expect("cancel continuations");

    assert_eq!(report.cancelled, 2);
    assert_eq!(report.skipped, 0);
    let statuses = store
        .continuations(&session_id)
        .expect("continuations")
        .into_iter()
        .map(|continuation| {
            (
                continuation.continuation_id,
                continuation.status,
                continuation.error,
            )
        })
        .collect::<Vec<_>>();
    assert!(statuses.iter().any(|(id, status, error)| {
        id == &queued.continuation_id
            && *status == SessionContinuationStatus::Cancelled
            && error.as_deref() == Some("operator requested cancel")
    }));
    assert!(statuses.iter().any(|(id, status, error)| {
        id == &running.continuation_id
            && *status == SessionContinuationStatus::Cancelled
            && error.as_deref() == Some("operator requested cancel")
    }));
    let cancel_events = store
        .agent_events(&session_id)
        .expect("cancel events")
        .into_iter()
        .filter(|event| matches!(event.kind, AgentEventKind::ContinuationCancelled))
        .collect::<Vec<_>>();
    assert_eq!(cancel_events.len(), 2);
    assert!(cancel_events.iter().any(|event| {
        event.payload["continuation_id"] == queued.continuation_id.as_str()
            && event.payload["reason"] == "operator requested cancel"
            && event.payload["status"] == "cancelled"
    }));
    assert!(cancel_events.iter().any(|event| {
        event.payload["continuation_id"] == running.continuation_id.as_str()
            && event.payload["reason"] == "operator requested cancel"
            && event.payload["status"] == "cancelled"
    }));
}

#[test]
fn workbench_continuations_json_line_exports_redacted_queue_actions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("continuation-json-session");
    let mut running_input =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::NextTurn);
    running_input.payload =
        serde_json::json!({"prompt": "continue", "api_key": "sk-continuation-secret"});
    let running = store
        .enqueue_continuation(&running_input)
        .expect("running continuation");
    store
        .claim_next_continuation(
            &SessionContinuationClaim::for_session(session_id.clone())
                .with_lease_owner("test-worker")
                .with_lease_duration_seconds(30),
        )
        .expect("claim")
        .expect("claimed continuation");
    let queued = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::ToolResult,
        ))
        .expect("queued continuation");
    let continuations = store.continuations(&session_id).expect("continuations");

    let line = continuations_json_line(&continuations);

    assert!(line.contains("continuations_json:"));
    assert!(line.contains("\"schema\":\"ikaros-workbench-continuations-v1\""));
    assert!(line.contains("\"queued\":1"));
    assert!(line.contains("\"running\":1"));
    assert!(line.contains("\"status\":\"running\""));
    assert!(line.contains("\"status\":\"queued\""));
    assert!(line.contains(&format!(
        "\"cancel\":\"/cancel {}\"",
        running.continuation_id
    )));
    assert!(line.contains(&format!(
        "\"cancel\":\"/cancel {}\"",
        queued.continuation_id
    )));
    assert!(line.contains("[REDACTED_SECRET]"));
    assert!(!line.contains("sk-continuation-secret"));
}

#[test]
fn workbench_cancel_selected_screen_continuation_uses_side_panel_selection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteSessionStore::new(temp.path());
    let session_id = SessionId::from("screen-cancel-session");
    let first = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::NextTurn,
        ))
        .expect("first continuation");
    let second = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::ToolResult,
        ))
        .expect("second continuation");
    let pending_inputs = VecDeque::from(["queued input".to_owned()]);

    let skipped = cancel_selected_screen_continuation(
        &store,
        &session_id,
        1,
        &pending_inputs,
        1,
        "screen selected cancel",
    )
    .expect("skip non-continuation row");
    assert_eq!(skipped.continuation_id, None);
    assert_eq!(skipped.report.cancelled, 0);

    let cancelled = cancel_selected_screen_continuation(
        &store,
        &session_id,
        1,
        &pending_inputs,
        2,
        "screen selected cancel",
    )
    .expect("cancel selected continuation");
    assert_eq!(
        cancelled.continuation_id.as_deref(),
        Some(first.continuation_id.as_str())
    );
    assert_eq!(cancelled.report.cancelled, 1);
    assert_eq!(cancelled.report.skipped, 0);

    let continuations = store
        .continuations(&session_id)
        .expect("continuations after selected cancel");
    assert!(continuations.iter().any(|continuation| {
        continuation.continuation_id == first.continuation_id
            && continuation.status == SessionContinuationStatus::Cancelled
    }));
    assert!(continuations.iter().any(|continuation| {
        continuation.continuation_id == second.continuation_id
            && continuation.status == SessionContinuationStatus::Queued
    }));
}

#[test]
fn interactive_live_cells_compact_model_stream_deltas_without_leaking_text() {
    let session_id = SessionId::from("live-session");
    let turn_id = TurnId::from("live-turn");
    let mut events = vec![chat_event(
        &session_id,
        &turn_id,
        "event-start",
        AgentEventKind::ModelStream(ModelStreamEvent::Start {
            provider: "mock".into(),
            model: "mock-ikaros".into(),
        }),
    )];
    for index in 0..12 {
        events.push(chat_event(
            &session_id,
            &turn_id,
            &format!("event-delta-{index}"),
            AgentEventKind::ModelStream(ModelStreamEvent::TextDelta(format!(
                "chunk-{index} sk-secret-value"
            ))),
        ));
    }
    events.push(chat_event(
        &session_id,
        &turn_id,
        "event-usage",
        AgentEventKind::ModelStream(ModelStreamEvent::Usage(TokenUsage {
            prompt_tokens: Some(10),
            completion_tokens: Some(12),
            total_tokens: Some(22),
            cache_read_tokens: None,
            cache_write_tokens: None,
        })),
    ));
    events.push(chat_event(
        &session_id,
        &turn_id,
        "event-done",
        AgentEventKind::ModelStream(ModelStreamEvent::Done),
    ));
    events.push(chat_event(
        &session_id,
        &turn_id,
        "event-tool",
        AgentEventKind::ToolCallCompleted,
    ));
    let refs = events.iter().collect::<Vec<_>>();

    let cells = super::compact_live_event_cells(&refs);
    let rendered = cells
        .iter()
        .map(crate::chat::workbench::WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(cells.len(), 3);
    assert!(rendered.contains("title=model stream summary"));
    assert!(rendered.contains("text_delta_chunks=12"));
    assert!(rendered.contains("usage_total=22"));
    assert!(rendered.contains("done=true"));
    assert!(rendered.contains("title=tool progress summary"));
    assert!(rendered.contains("completed=1"));
    assert!(rendered.contains("cell kind=tool"));
    assert!(!rendered.contains("chunk-"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn interactive_live_cells_json_line_exports_redacted_cell_payload() {
    let session_id = SessionId::from("live-json-session");
    let turn_id = TurnId::from("live-json-turn");
    let events = [
        chat_event(
            &session_id,
            &turn_id,
            "event-start",
            AgentEventKind::ModelStream(ModelStreamEvent::Start {
                provider: "mock".into(),
                model: "mock-ikaros".into(),
            }),
        ),
        chat_event(
            &session_id,
            &turn_id,
            "event-delta",
            AgentEventKind::ModelStream(ModelStreamEvent::TextDelta(
                "secret sk-secret-value".into(),
            )),
        ),
        chat_event(
            &session_id,
            &turn_id,
            "event-tool",
            AgentEventKind::ToolCallFailed,
        ),
    ];
    let refs = events.iter().collect::<Vec<_>>();
    let visible_events = refs
        .iter()
        .copied()
        .filter(|event| super::default_live_cell_event(&event.kind))
        .collect::<Vec<_>>();
    let cells = super::compact_live_event_cells(&refs);

    let line = super::live_cells_json_line(&refs, &visible_events, &cells);
    let payload = line
        .strip_prefix("live_cells_json: ")
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .expect("live cells JSON payload");

    assert_eq!(payload["schema"], "ikaros-workbench-live-cells-v1");
    assert_eq!(payload["version"], 1);
    assert_eq!(payload["total_events"], 3);
    assert_eq!(payload["counts"]["model"], 2);
    assert_eq!(payload["counts"]["tool"], 1);
    assert_eq!(payload["model_stream_suppressed"], 2);
    assert_eq!(payload["cells"][0]["kind"], "model");
    assert_eq!(payload["cells"][0]["title"], "model stream summary");
    assert_eq!(payload["cells"][1]["kind"], "tool");
    assert_eq!(payload["cells"][1]["title"], "tool progress summary");
    let serialized = serde_json::to_string(&payload).expect("serialize payload");
    assert!(!serialized.contains("sk-secret-value"));
    assert!(!serialized.contains("secret sk-"));
}

#[test]
fn interactive_human_activity_lines_render_codex_like_tool_rows_without_machine_fields() {
    let session_id = SessionId::from("human-live-session");
    let turn_id = TurnId::from("human-live-turn");
    let mut event = chat_event(
        &session_id,
        &turn_id,
        "event-tool",
        AgentEventKind::ToolCallCompleted,
    );
    event.payload = serde_json::json!({
        "name": "read_file",
        "status": "completed",
        "summary": "Read SKILL.md with token sk-secret-value",
    });

    let lines = super::live::human_activity_lines(&event).expect("human activity");
    let rendered = lines.join("\n");

    assert!(rendered.contains("• Explored"));
    assert!(rendered.contains("└ Read SKILL.md"));
    assert!(!rendered.contains("status="));
    assert!(!rendered.contains("kind="));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn interactive_human_activity_lines_hide_context_by_default_but_keep_debug_context() {
    let session_id = SessionId::from("human-context-session");
    let turn_id = TurnId::from("human-context-turn");
    let mut event = chat_event(
        &session_id,
        &turn_id,
        "event-context",
        AgentEventKind::ContextDiff,
    );
    event.payload = serde_json::json!({
        "sections": [{ "kind": "history" }, { "kind": "memory" }],
        "references": [],
    });

    assert!(super::live::human_activity_lines_with_debug_context(&event, false).is_none());
    let lines = super::live::human_activity_lines_with_debug_context(&event, true)
        .expect("debug context activity");

    assert_eq!(
        lines,
        vec![
            "• Gathered context".to_owned(),
            "  • 2 section(s), 0 reference(s)".to_owned(),
        ]
    );
}

#[test]
fn interactive_human_activity_lines_render_errors_without_protocol_fields() {
    let session_id = SessionId::from("human-error-session");
    let turn_id = TurnId::from("human-error-turn");
    let mut event = chat_event(&session_id, &turn_id, "event-error", AgentEventKind::Error);
    event.payload = serde_json::json!({
        "phase": "memory_sync",
        "message": "failed with token=sk-secret-value",
        "recoverable": true,
    });

    let lines = super::live::human_activity_lines(&event).expect("human error activity");
    let rendered = lines.join("\n");

    assert!(rendered.contains("• Error"));
    assert!(rendered.contains("[REDACTED_SECRET]"));
    assert!(!rendered.contains("phase="));
    assert!(!rendered.contains("recoverable"));
    assert!(!rendered.contains("sk-secret-value"));
}

#[test]
fn interactive_live_cells_include_context_progress_summary() {
    let session_id = SessionId::from("live-context-session");
    let turn_id = TurnId::from("live-context-turn");
    let mut context_event = chat_event(
        &session_id,
        &turn_id,
        "event-context",
        AgentEventKind::ContextDiff,
    );
    context_event.payload = serde_json::json!({
        "sections": [{ "kind": "history", "estimated_tokens": 12 }],
        "references": [{ "raw": "@file:src/lib.rs" }],
        "budget": {
            "used_tokens": 128,
            "max_tokens": 512,
            "context_window": 4096,
            "estimator": "heuristic-v1",
        },
    });
    let refs = vec![&context_event];

    let default_cells = super::live::compact_live_event_cells_with_debug(&refs, false);
    assert!(default_cells.is_empty());

    let cells = super::live::compact_live_event_cells_with_debug(&refs, true);
    let rendered = cells
        .iter()
        .map(crate::chat::workbench::WorkbenchCell::render)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(rendered.contains("title=context progress summary"));
    assert!(rendered.contains("diffs=1"));
    assert!(rendered.contains("latest_sections=1"));
    assert!(rendered.contains("latest_references=1"));
    assert!(rendered.contains("used=128"));
    assert!(rendered.contains("context_window=4096"));
    assert!(rendered.contains("trace=/trace --kind context"));
    assert!(rendered.contains("context=/context"));
}

#[test]
fn interactive_default_live_cell_events_hide_internal_and_process_events() {
    assert!(super::default_live_cell_event(
        &AgentEventKind::ToolCallCompleted
    ));
    assert!(super::default_live_cell_event(
        &AgentEventKind::ToolCallFailed
    ));
    assert!(super::default_live_cell_event(
        &AgentEventKind::ToolCallCancelled
    ));
    assert!(super::default_live_cell_event(
        &AgentEventKind::ApprovalRequested
    ));
    assert!(super::default_live_cell_event(&AgentEventKind::CodingTurn));
    assert!(super::default_live_cell_event(&AgentEventKind::Error));

    assert!(!super::default_live_cell_event(&AgentEventKind::TurnStart));
    assert!(!super::default_live_cell_event(&AgentEventKind::TurnEnd));
    assert!(!super::default_live_cell_event(
        &AgentEventKind::ContextDiff
    ));
    assert!(!super::default_live_cell_event(
        &AgentEventKind::ContextCompacted
    ));
    assert!(!super::default_live_cell_event(
        &AgentEventKind::MemoryLifecycle
    ));
    assert!(!super::default_live_cell_event(
        &AgentEventKind::ToolCallStarted
    ));
    assert!(!super::default_live_cell_event(
        &AgentEventKind::ToolCallOutputDelta
    ));
    assert!(!super::default_live_cell_event(
        &AgentEventKind::ApprovalResolved
    ));
}

#[test]
fn workbench_live_event_sink_updates_snapshots_while_events_are_emitted() {
    let session_id = SessionId::from("live-sink-session");
    let turn_id = TurnId::from("live-sink-turn");
    let sink = super::live::WorkbenchLiveEventSink::default();

    sink.emit(&chat_event(
        &session_id,
        &turn_id,
        "event-start",
        AgentEventKind::ModelStream(ModelStreamEvent::Start {
            provider: "mock".into(),
            model: "mock-live".into(),
        }),
    ))
    .expect("emit start");
    sink.emit(&chat_event(
        &session_id,
        &turn_id,
        "event-secret-delta",
        AgentEventKind::ModelStream(ModelStreamEvent::TextDelta(
            "streamed sk-secret-value fragment".into(),
        )),
    ))
    .expect("emit delta");
    sink.emit(&chat_event(
        &session_id,
        &turn_id,
        "event-tool",
        AgentEventKind::ToolCallCompleted,
    ))
    .expect("emit tool");

    let snapshots = sink.snapshots().expect("snapshots");
    assert_eq!(snapshots.len(), 3);
    assert!(snapshots[0].contains("live_cells: 1 total_events=1"));
    assert!(snapshots[1].contains("model_stream_suppressed=2"));
    assert!(snapshots[2].contains("cell kind=tool"));
    let joined = snapshots.join("\n");
    assert!(!joined.contains("sk-secret-value"));
    assert!(!joined.contains("streamed sk-"));
}

fn chat_event(
    session_id: &SessionId,
    turn_id: &TurnId,
    event_id: &str,
    kind: AgentEventKind,
) -> AgentEvent {
    AgentEvent {
        event_id: EventId::from(event_id),
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        parent_event_id: None,
        at: time::OffsetDateTime::now_utc(),
        source: AgentEventSource::Runtime,
        kind,
        payload: serde_json::Value::Null,
    }
}
