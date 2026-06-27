// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use crate::support::TestHome;
use ikaros_gateway::{
    GatewayDeliveryStatus, GatewayMessage, GatewayMessageKind, GatewayMessageStatus, GatewayRoute,
    LocalGatewayStore,
};
use ikaros_harness::{AuditEvent, AuditLog};
use ikaros_memory::{
    JsonlMemoryCandidateStore, JsonlMemoryJournal, JsonlMemoryStore, JsonlWorkingMemoryStore,
    MemoryCandidate, MemoryCandidateReason, MemoryCandidateStatus, MemoryJournal,
    MemoryJournalAction, MemoryKind, MemoryRecord, MemoryStore, WorkingMemoryRecord,
};
use ikaros_models::{ModelUsageLedger, ModelUsageRecord};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSource, SessionContinuationClaim,
    SessionContinuationInput, SessionContinuationKind, SessionEntry, SessionEntryKind, SessionId,
    SessionRecord, SessionSource, SessionStore, SqliteSessionStore, TurnId,
};
use serde_json::json;

#[test]
fn init_doctor_chat_and_task_dry_run_work_with_explicit_offline_mock_config() {
    let env = TestHome::new();
    let init = env.init();
    assert!(init.contains("Ikaros initialized"));
    assert!(env.home.join("config.yaml").exists());
    assert!(env.home.join("persona.md").exists());
    env.use_offline_mock_config();

    let doctor = env.run(["doctor"]);
    assert!(doctor.contains("model: provider=mock"));
    assert!(doctor.contains("agent_profiles:"));
    assert!(doctor.contains("memory_providers: local=local-jsonl external_active=none"));
    assert!(doctor.contains("gateway: inbox="));

    let chat = env.run([
        "chat",
        "--message",
        "hello smoke token=abc123",
        "--no-context",
        "--context-token-budget",
        "64",
    ]);
    assert!(chat.contains("ok: true"));
    assert!(chat.contains("provider: mock"));
    assert!(chat.contains("emotion: Satisfied"));
    assert!(chat.contains("model_usage:"));
    assert!(chat.contains("session_state_db:"));
    assert!(chat.contains("chat_timeline: session_store"));
    assert!(!chat.contains("\nchat_history:"));
    assert!(!chat.contains("abc123"));
    let session_id = chat
        .lines()
        .find_map(|line| line.strip_prefix("chat_session: "))
        .expect("chat session id");

    let chat_next = env.run([
        "chat",
        "--message",
        "continue smoke token=abc123",
        "--chat-session",
        session_id,
        "--history-context-limit",
        "1",
        "--history-summary-limit",
        "2",
    ]);
    assert!(chat_next.contains("context: relationship=0 references=0 history=1 memory=0 rag=0"));

    let history = env.run(["chat", "--history", "--history-limit", "2"]);
    assert!(history.contains("records: 2"));
    assert!(history.contains("provider=mock"));
    assert!(history.contains("[REDACTED_SECRET]"));
    assert!(!history.contains("abc123"));
    let session_history = env.run(["chat", "--history", "--history-session", session_id]);
    assert!(session_history.contains(&format!("session: {session_id}")));
    assert!(session_history.contains("records: 2"));

    let sessions = env.run(["chat", "--sessions", "--history-limit", "5"]);
    assert!(sessions.contains("sessions: 1"));
    assert!(sessions.contains(&format!("session={session_id}")));
    assert!(sessions.contains("turns=2"));
    assert!(sessions.contains(&format!(
        "continue: ikaros chat --chat-session {session_id} --message"
    )));
    assert!(sessions.contains("[REDACTED_SECRET]"));
    assert!(!sessions.contains("abc123"));

    let search = env.run([
        "chat",
        "--history-search",
        "hello smoke token=abc123",
        "--history-limit",
        "5",
    ]);
    assert!(search.contains("query: hello smoke token=[REDACTED_SECRET]"));
    assert!(search.contains("records: 1"));
    assert!(search.contains("matches:"));
    assert!(search.contains("[REDACTED_SECRET]"));
    assert!(!search.contains("abc123"));

    let session_search = env.run([
        "chat",
        "--history-search",
        "hello",
        "--history-session",
        session_id,
    ]);
    assert!(session_search.contains(&format!("session: {session_id}")));
    assert!(session_search.contains("records: 1"));

    let usage = fs::read_to_string(env.home.join("audit/model-usage.jsonl")).expect("usage");
    assert!(!usage.contains("hello smoke"));
    assert!(!env.home.join("chat/history.jsonl").exists());

    let deleted_session = env.run(["chat", "--history-delete-session", session_id]);
    assert!(deleted_session.contains(&format!("deleted_session: {session_id}")));
    assert!(deleted_session.contains("deleted_records: 0"));
    assert!(deleted_session.contains("deleted_session_replay: true"));
    let cleared = env.run(["chat", "--history-clear"]);
    assert!(cleared.contains("deleted_records: 0"));
    let empty_history = env.run(["chat", "--history"]);
    assert!(empty_history.contains("records: 0"));

    let task = env.run(["task", "run", "--dry-run", "Summarize the smoke workspace"]);
    assert!(task.contains("dry_run: true"));
    assert!(task.contains("state: Completed"));
    assert!(task.contains("dry-run allowed task_summarize"));

    let loop_task = env.run([
        "task",
        "run",
        "--dry-run",
        "--agent-loop",
        "Summarize the smoke workspace through the agent loop",
    ]);
    assert!(loop_task.contains("dry_run: true"));
    assert!(loop_task.contains("agent_loop: true"));
    assert!(loop_task.contains("state: Completed"));
    assert!(loop_task.contains("loop: stop=FinalAnswer"));

    let loop_agent = env.run([
        "agent",
        "run",
        "--profile",
        "build",
        "--dry-run",
        "--agent-loop",
        "Inspect the smoke workspace through the agent loop",
    ]);
    assert!(loop_agent.contains("\"agent_loop\": true"));
    assert!(loop_agent.contains("\"loop_report\":"));
    assert!(loop_agent.contains("\"state\": \"Completed\""));
}

#[test]
fn setup_writes_usable_model_config_without_printing_secret() {
    let env = TestHome::new();

    let setup = env.run([
        "setup",
        "--api-key",
        "test-provider-key",
        "--base-url",
        "https://api.example.test/v1",
        "--model",
        "test-chat-model",
    ]);

    assert!(setup.contains("Ikaros setup"));
    assert!(setup.contains("config_created: true"));
    assert!(setup.contains("model_provider: openai-compatible"));
    assert!(setup.contains("model_model: test-chat-model"));
    assert!(setup.contains("model_api_key_configured: true"));
    assert!(setup.contains("embedding_provider: hash"));
    assert!(setup.contains("tts_provider: mock"));
    assert!(setup.contains("asr_provider: mock"));
    assert!(!setup.contains("test-provider-key"));

    let config = fs::read_to_string(env.home.join("config.yaml")).expect("config");
    assert!(config.contains(r#"api_key: "test-provider-key""#));
    assert!(config.contains(r#"base_url: "https://api.example.test/v1""#));
    assert!(config.contains(r#"model: "test-chat-model""#));
    assert!(config.contains(r#"provider: "openai-compatible""#));
    assert!(config.contains(r#"transport: "openai-compatible-chat-completions""#));
    assert!(config.contains("embedding_provider: \"hash\""));
    assert!(config.contains("provider: \"mock\""));

    let validate = env.run(["config", "validate"]);
    assert!(validate.contains("config valid:"));

    let doctor = env.run(["doctor"]);
    assert!(
        doctor.contains(
            "model: provider=openai-compatible model=test-chat-model key_configured=true"
        )
    );
    assert!(doctor.contains("rag: backend=jsonl embedding_provider=hash"));
    assert!(doctor.contains("voice: tts_provider=mock"));
}

#[test]
fn doctor_uses_agent_instance_model_and_provider_overrides() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"
schema_version: 1

providers:
  model:
    api_key: ""
    base_url: ""

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: global-mock

agent:
  default: build
  instances:
    coder:
      profile: build
      providers:
        model:
          api_key: ""
          base_url: ""
      model:
        provider: mock
        runtime: harness-agent-loop
        transport: mock
        model: instance-mock

rag:
  backend: jsonl
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
    .expect("write instance override config");

    let global = env.run(["doctor"]);
    assert!(global.contains("model: provider=mock model=global-mock"));
    assert!(global.contains("agent: build mode=build"));
    let global_chat = env.run(["chat", "--message", "global model smoke", "--no-context"]);
    assert!(global_chat.contains("provider: mock"));
    assert!(global_chat.contains("model: global-mock"));

    let instance = env.run(["--agent", "coder", "doctor"]);
    assert!(instance.contains("agent: coder mode=build"));
    assert!(instance.contains("model: provider=mock model=instance-mock"));
    let instance_chat = env.run([
        "--agent",
        "coder",
        "chat",
        "--message",
        "instance model smoke",
        "--no-context",
    ]);
    assert!(instance_chat.contains("provider: mock"));
    assert!(instance_chat.contains("model: instance-mock"));
}

#[test]
fn doctor_reports_config_validation_issues_without_leaking_secrets() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"
schema_version: 1
providers:
  model:
    api_key: sk-local-secret
    base_url: https://api.example/v1

model:
  default:
    provider: openai-compatible
    runtime: harness-agent-loop
    transport: openai-compatible-chat-completions
    model: ""

rag:
  backend: jsonl
  embedding_provider: hash

voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr
"#,
    )
    .expect("write invalid runtime config");

    let doctor = env.run(["doctor"]);

    assert!(doctor.contains("config_valid: false"));
    assert!(doctor.contains("config_issue: error: model.default.model: must not be empty"));
    assert!(!doctor.contains("sk-local-secret"));
    assert!(!doctor.contains("https://api.example/v1"));
}

#[test]
fn agent_instance_toolset_override_limits_model_visible_skills() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"
schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

agent:
  default: build
  instances:
    restricted:
      profile: build
      toolsets: [core, workspace, memory]

rag:
  embedding_provider: hash

voice:
  tts:
    provider: mock
  asr:
    provider: mock
"#,
    )
    .expect("write toolset override config");

    let default_coding = env.run(["skill", "inspect", "code_workflow"]);
    assert!(default_coding.contains("toolset: coding"));
    assert!(default_coding.contains("model_visibility: deferred"));

    let restricted_coding = env.run(["--agent", "restricted", "skill", "inspect", "code_workflow"]);
    assert!(restricted_coding.contains("toolset: coding"));
    assert!(restricted_coding.contains("model_visibility: disabled"));

    let restricted_bridge = env.run(["--agent", "restricted", "skill", "inspect", "tool_search"]);
    assert!(restricted_bridge.contains("toolset: core"));
    assert!(restricted_bridge.contains("model_visibility: direct"));
}

#[test]
fn chat_session_history_uses_session_replay_as_authority() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let chat = env.run([
        "chat",
        "--message",
        "session replay history token=abc123",
        "--no-context",
    ]);
    let session_id = chat
        .lines()
        .find_map(|line| line.strip_prefix("chat_session: "))
        .expect("chat session id");
    assert!(!env.home.join("chat/history.jsonl").exists());

    let global_history = env.run(["chat", "--history", "--history-limit", "5"]);
    assert!(global_history.contains("history_source: session_replay"));
    assert!(global_history.contains("history_authority: session_store"));
    assert!(global_history.contains("records: 1"));
    assert!(global_history.contains("[REDACTED_SECRET]"));
    assert!(!global_history.contains("abc123"));

    let global_search = env.run(["chat", "--history-search", "session replay history"]);
    assert!(global_search.contains("history_source: session_replay"));
    assert!(global_search.contains("history_authority: session_store"));
    assert!(global_search.contains("records: 1"));
    assert!(global_search.contains("[REDACTED_SECRET]"));
    assert!(!global_search.contains("abc123"));

    let history = env.run(["chat", "--history", "--history-session", session_id]);
    assert!(history.contains(&format!("session: {session_id}")));
    assert!(history.contains("history_source: session_replay"));
    assert!(history.contains("history_authority: session_store"));
    assert!(history.contains("records: 1"));
    assert!(history.contains("[REDACTED_SECRET]"));
    assert!(!history.contains("abc123"));

    let search = env.run([
        "chat",
        "--history-search",
        "session replay history",
        "--history-session",
        session_id,
    ]);
    assert!(search.contains(&format!("session: {session_id}")));
    assert!(search.contains("history_source: session_replay"));
    assert!(search.contains("history_authority: session_store"));
    assert!(search.contains("records: 1"));
    assert!(search.contains("[REDACTED_SECRET]"));
    assert!(!search.contains("abc123"));

    let sessions = env.run(["chat", "--sessions", "--history-limit", "5"]);
    assert!(sessions.contains("history_source: session_replay"));
    assert!(sessions.contains("history_authority: session_store"));
    assert!(sessions.contains("sessions: 1"));
    assert!(sessions.contains(&format!("session={session_id}")));
    assert!(sessions.contains("turns=1"));
    assert!(sessions.contains("[REDACTED_SECRET]"));
    assert!(!sessions.contains("abc123"));

    let deleted = env.run(["chat", "--history-delete-session", session_id]);
    assert!(deleted.contains(&format!("deleted_session: {session_id}")));
    assert!(deleted.contains("deleted_session_replay: true"));

    let hidden_history = env.run(["chat", "--history", "--history-session", session_id]);
    assert!(hidden_history.contains("records: 0"));
    assert!(!hidden_history.contains("session replay history"));

    let hidden_sessions = env.run(["chat", "--sessions", "--history-limit", "5"]);
    assert!(hidden_sessions.contains("sessions: 0"));
    assert!(!hidden_sessions.contains(session_id));

    let clear_session_id = "clear-replay-history-session";
    let clear_chat = env.run([
        "chat",
        "--message",
        "clear replay history token=abc123",
        "--chat-session",
        clear_session_id,
        "--no-context",
    ]);
    assert!(clear_chat.contains(&format!("chat_session: {clear_session_id}")));
    let cleared = env.run(["chat", "--history-clear"]);
    assert!(cleared.contains("deleted_session_replay_sessions: 1"));

    let cleared_history = env.run(["chat", "--history", "--history-session", clear_session_id]);
    assert!(cleared_history.contains("records: 0"));
    assert!(!cleared_history.contains("clear replay history"));

    let cleared_sessions = env.run(["chat", "--sessions", "--history-limit", "5"]);
    assert!(cleared_sessions.contains("sessions: 0"));
    assert!(!cleared_sessions.contains(clear_session_id));
}

#[test]
fn skill_visibility_distinguishes_direct_deferred_and_disabled_toolsets() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let default_rag = env.run(["skill", "inspect", "rag_search"]);
    assert!(default_rag.contains("toolset: rag"));
    assert!(default_rag.contains("model_visibility: deferred"));

    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  backend: jsonl
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

agent:
  default: build
  profiles:
    build:
      toolsets: [core, workspace, memory]
"#,
    )
    .expect("write restricted config");

    let restricted_rag = env.run(["skill", "inspect", "rag_search"]);
    assert!(restricted_rag.contains("toolset: rag"));
    assert!(restricted_rag.contains("model_visibility: disabled"));

    let direct_bridge = env.run(["skill", "inspect", "tool_search"]);
    assert!(direct_bridge.contains("toolset: core"));
    assert!(direct_bridge.contains("model_visibility: direct"));
}

#[test]
fn local_memory_and_message_gateway_smoke_paths_run_offline() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let added = env.run([
        "memory",
        "add",
        "--kind",
        "project",
        "--scope",
        "smoke",
        "Ikaros smoke memory",
    ]);
    assert!(added.contains("summary: memory appended"));
    assert!(added.contains("\"backend\": \"jsonl\""));

    let search = env.run([
        "memory", "search", "--kind", "project", "--scope", "smoke", "Ikaros",
    ]);
    assert!(search.contains("Ikaros smoke memory"));
    assert!(search.contains("\"scope\": \"smoke\""));

    let providers = env.run(["memory", "provider", "list"]);
    assert!(providers.contains("\"id\": \"local-jsonl\""));
    assert!(providers.contains("\"external\": []"));

    let active_provider = env.run(["memory", "provider", "active"]);
    assert!(active_provider.contains("\"local\""));
    assert!(active_provider.contains("\"external\": null"));

    let shown_provider = env.run(["memory", "provider", "show", "local-jsonl"]);
    assert!(shown_provider.contains("\"kind\": \"builtin_local\""));

    let sent = env.run([
        "message",
        "send",
        "--kind",
        "task",
        "summarize smoke gateway",
    ]);
    assert!(sent.contains("enqueued:"));
    assert!(sent.contains("\"kind\": \"Task\""));

    let drained = env.run(["message", "drain", "--dry-run"]);
    assert!(drained.contains("\"status\": \"Pending\""));
    assert!(drained.contains("gateway_inbox:"));
    assert!(drained.contains("gateway_outbox:"));
}

#[test]
fn gateway_status_exposes_thread_resume_without_secret_leak() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    fs::create_dir_all(env.home.join("gateway")).expect("gateway dir");
    fs::write(
        env.home.join("gateway/message-worker.lock"),
        "pid=999999999\nowner=worker-token=abc123\nstarted_at=2026-06-23T00:00:00Z\n",
    )
    .expect("worker lock");
    fs::write(
        env.home.join("gateway/message-worker-events.jsonl"),
        r#"{"schema":"ikaros-message-worker-forensics-v1","version":1,"run_id":"old","event":"started","status":"running","at":"2026-06-23T00:00:00Z","pid":122}
{"schema":"ikaros-message-worker-forensics-v1","version":1,"run_id":"old","event":"stopped","status":"failed","at":"2026-06-23T00:01:00Z","pid":122,"reason":"failed token=abc123"}
"#,
    )
    .expect("worker events");

    let sent = env.run([
        "message",
        "send",
        "--kind",
        "chat",
        "--source",
        "telegram",
        "--account",
        "account-1",
        "--peer",
        "peer-1",
        "--thread",
        "thread-1",
        "--message-id",
        "message-1",
        "--idempotency-key",
        "idem-token=abc123",
        "gateway resume token=abc123",
    ]);
    assert!(sent.contains("enqueued:"));
    assert!(!sent.contains("abc123"));

    let status = env.run(["message", "status"]);
    assert!(status.contains("gateway_status:"));
    assert!(status.contains("gateway_sessions: 1"));
    assert!(status.contains("gateway_dead_lettered: 0"));
    assert!(status.contains("gateway_worker_lock: present=true"));
    assert!(status.contains("stale=true"));
    assert!(status.contains("message-worker.lock"));
    assert!(status.contains("owner=pid=999999999 owner=[REDACTED_SECRET]"));
    assert!(status.contains("gateway_worker_forensics: latest_event=stopped"));
    assert!(status.contains("latest_status=failed"));
    assert!(status.contains("reason=failed token=[REDACTED_SECRET]"));
    assert!(status.contains("gateway_session:"));
    assert!(status.contains("source=telegram"));
    assert!(status.contains("thread=thread-1"));
    assert!(status.contains("resume: ikaros chat --chat-session gateway-"));
    assert!(!status.contains("abc123"));

    let workbench = env.run_with_stdin(
        ["chat", "--chat-session", "gateway-status-workbench"],
        "/gateway\n/quit\n",
    );
    assert!(workbench.contains("gateway_sessions: 1"));
    assert!(
        workbench.contains(
            "gateway_worker: processing=0 stale_processing=0 retryable=0 dead_lettered=0"
        )
    );
    assert!(workbench.contains("gateway_worker_lock: present=true"));
    assert!(workbench.contains("stale=true"));
    assert!(workbench.contains("gateway_worker_forensics: latest_event=stopped"));
    assert!(workbench.contains("gateway_session:"));
    assert!(workbench.contains("resume: ikaros chat --chat-session gateway-"));
    assert!(!workbench.contains("abc123"));
}

#[test]
fn gateway_status_reports_worker_lease_and_dead_letter_without_secret_leak() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    let store = LocalGatewayStore::new(env.home.join("gateway"));
    let processing = store
        .enqueue(GatewayRoute::new(
            "worker",
            GatewayMessageKind::Task,
            "processing token=abc123",
            None,
        ))
        .expect("processing");
    store
        .claim_pending_with_owner(1, "worker-token=abc123")
        .expect("claim");
    let retry = store
        .enqueue(GatewayRoute::new(
            "worker",
            GatewayMessageKind::Task,
            "retry token=abc123",
            None,
        ))
        .expect("retry");
    let retry_claim = store
        .claim_pending_with_owner(1, "retry-worker")
        .expect("retry claim")
        .pop()
        .expect("retry claimed");
    store
        .record_failure_for_claim(&retry_claim, "retry failed token=abc123", 2)
        .expect("retry failure");
    rewrite_gateway_messages(&store, |messages| {
        let processing = messages
            .iter_mut()
            .find(|message| message.id == processing.id)
            .expect("processing message");
        processing.lease_expires_at = Some("2000-01-01T00:00:00Z".into());
    });
    let dead = store
        .enqueue(GatewayRoute::new(
            "worker",
            GatewayMessageKind::Task,
            "dead token=abc123",
            None,
        ))
        .expect("dead");
    store
        .record_status(
            &dead.id,
            GatewayMessageStatus::DeadLettered,
            "failed token=abc123",
        )
        .expect("failure");
    let retry_delivery = store
        .deliver(
            "message-one",
            "chat_response",
            "retry delivery token=abc123",
        )
        .expect("retry delivery");
    let retry_delivery_claim = store
        .claim_pending_deliveries_with_owner(1, "adapter-token=abc123")
        .expect("delivery claim")
        .pop()
        .expect("claimed delivery");
    assert_eq!(retry_delivery_claim.id, retry_delivery.id);
    store
        .record_delivery_failure_for_claim(
            &retry_delivery_claim,
            "delivery failed token=abc123",
            2,
            30,
        )
        .expect("delivery failure");
    let dead_delivery = store
        .deliver("message-two", "chat_response", "dead delivery token=abc123")
        .expect("dead delivery");
    let mut deliveries = store.deliveries().expect("deliveries");
    let dead_delivery = deliveries
        .iter_mut()
        .find(|delivery| delivery.id == dead_delivery.id)
        .expect("dead delivery listed");
    dead_delivery.status = GatewayDeliveryStatus::DeadLettered;
    dead_delivery.last_error = Some("terminal delivery token=abc123".into());
    rewrite_gateway_deliveries(&store, &deliveries);

    let status = env.run(["message", "status"]);
    assert!(status.contains("gateway_processing: 1"));
    assert!(status.contains("gateway_pending: 1"));
    assert!(status.contains("gateway_dead_lettered: 1"));
    assert!(
        status.contains(
            "gateway_worker: processing=1 stale_processing=1 retryable=1 dead_lettered=1"
        )
    );
    assert!(status.contains("attempts=1"));
    assert!(status.contains("lease_owner=worker-token=[REDACTED_SECRET]"));
    assert!(status.contains("stale=true"));
    assert!(status.contains("gateway_retryable_message:"));
    assert!(status.contains(&retry.id));
    assert!(status.contains("last_error=retry failed token=[REDACTED_SECRET]"));
    assert!(status.contains("gateway_dead_lettered_message:"));
    assert!(status.contains("dead_lettered=1"));
    assert!(status.contains(&processing.id));
    assert!(
        status.contains(
            "gateway_deliveries_status: pending=1 processing=0 delivered=0 dead_lettered=1"
        )
    );
    assert!(status.contains("gateway_retryable_delivery:"));
    assert!(status.contains("last_error=delivery failed token=[REDACTED_SECRET]"));
    assert!(status.contains("gateway_dead_lettered_delivery:"));
    assert!(status.contains("terminal delivery token=[REDACTED_SECRET]"));
    assert!(!status.contains("abc123"));
}

#[test]
fn gateway_delivery_cli_claim_fail_and_ack_are_lease_bound_and_redacted() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    let store = LocalGatewayStore::new(env.home.join("gateway"));
    let delivery = store
        .deliver("message-one", "chat_response", "deliver token=abc123")
        .expect("delivery");

    let claimed = env.run([
        "message",
        "delivery",
        "claim",
        "--limit",
        "1",
        "--owner",
        "adapter-token=abc123",
    ]);
    assert!(claimed.contains("\"status\": \"Processing\""));
    assert!(claimed.contains("\"attempt_count\": 1"));
    assert!(claimed.contains("adapter-token=[REDACTED_SECRET]"));
    assert!(!claimed.contains("abc123"));

    let failed = env.run([
        "message",
        "delivery",
        "fail",
        &delivery.id,
        "--lease-owner",
        "adapter-token=abc123",
        "--reason",
        "remote token=abc123",
        "--max-attempts",
        "2",
        "--backoff-seconds",
        "30",
    ]);
    assert!(failed.contains("message_delivery_failed: true"));
    assert!(failed.contains("status=Pending"));
    assert!(failed.contains("remote token=[REDACTED_SECRET]"));
    assert!(!failed.contains("abc123"));

    let blocked_by_backoff = env.run([
        "message",
        "delivery",
        "claim",
        "--limit",
        "1",
        "--owner",
        "adapter-b",
    ]);
    assert!(blocked_by_backoff.contains("message_delivery_claimed: 0"));

    let mut deliveries = store.deliveries().expect("deliveries");
    let retry = deliveries
        .iter_mut()
        .find(|candidate| candidate.id == delivery.id)
        .expect("retry delivery");
    retry.next_attempt_at = Some("2020-01-01T00:00:00Z".into());
    rewrite_gateway_deliveries(&store, &deliveries);

    let second_claim = env.run([
        "message",
        "delivery",
        "claim",
        "--limit",
        "1",
        "--owner",
        "adapter-b",
    ]);
    assert!(second_claim.contains("message_delivery_claimed: 1"));
    assert!(second_claim.contains("\"attempt_count\": 2"));

    let wrong_owner = env.run_failure([
        "message",
        "delivery",
        "ack",
        &delivery.id,
        "--lease-owner",
        "adapter-a",
        "--summary",
        "wrong owner",
    ]);
    assert!(wrong_owner.contains("delivery lease owner mismatch"));

    let ack = env.run([
        "message",
        "delivery",
        "ack",
        &delivery.id,
        "--lease-owner",
        "adapter-b",
        "--summary",
        "delivered token=abc123",
    ]);
    assert!(ack.contains("message_delivery_delivered: true"));
    assert!(ack.contains("status=Delivered"));
    assert!(ack.contains("delivered token=[REDACTED_SECRET]"));
    assert!(!ack.contains("abc123"));

    let final_delivery = store
        .deliveries()
        .expect("deliveries")
        .into_iter()
        .find(|candidate| candidate.id == delivery.id)
        .expect("final delivery");
    assert_eq!(final_delivery.status, GatewayDeliveryStatus::Delivered);
    assert!(final_delivery.delivered_at.is_some());
}

fn rewrite_gateway_messages(
    store: &LocalGatewayStore,
    update: impl FnOnce(&mut Vec<GatewayMessage>),
) {
    let mut messages = store.list().expect("gateway messages");
    update(&mut messages);
    let jsonl = messages
        .iter()
        .map(|message| serde_json::to_string(message).expect("message json"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(store.inbox_path(), format!("{jsonl}\n")).expect("rewrite gateway inbox");
}

fn rewrite_gateway_deliveries(
    store: &LocalGatewayStore,
    deliveries: &[ikaros_gateway::GatewayDelivery],
) {
    let jsonl = deliveries
        .iter()
        .map(|delivery| serde_json::to_string(delivery).expect("delivery json"))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(store.outbox_path(), format!("{jsonl}\n")).expect("rewrite gateway outbox");
}

#[test]
fn memory_cli_filters_observer_subject_perspective() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let alice = env.run([
        "memory",
        "add",
        "--kind",
        "relationship",
        "--scope",
        "default",
        "--observer",
        "alice",
        "--subject",
        "bob",
        "Bob likes pancakes",
    ]);
    assert!(alice.contains("summary: memory appended"));
    let carol = env.run([
        "memory",
        "add",
        "--kind",
        "relationship",
        "--scope",
        "default",
        "--observer",
        "carol",
        "--subject",
        "bob",
        "Bob prefers waffles",
    ]);
    assert!(carol.contains("summary: memory appended"));

    let alice_search = env.run([
        "memory",
        "search",
        "--kind",
        "relationship",
        "--scope",
        "default",
        "--observer",
        "alice",
        "--subject",
        "bob",
        "Bob",
    ]);
    assert!(alice_search.contains("pancakes"));
    assert!(!alice_search.contains("waffles"));

    let carol_list = env.run([
        "memory",
        "list",
        "--kind",
        "relationship",
        "--scope",
        "default",
        "--observer",
        "carol",
        "--subject",
        "bob",
    ]);
    assert!(carol_list.contains("waffles"));
    assert!(!carol_list.contains("pancakes"));
}

#[test]
fn memory_projection_and_candidate_cli_manage_core_memory_surface() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    let store = JsonlMemoryStore::new(env.home.join("memory"));
    store
        .append(
            MemoryRecord::new(
                MemoryKind::User,
                "default",
                "User preference: concise updates",
            )
            .expect("user memory"),
        )
        .expect("append user");
    store
        .append(
            MemoryRecord::new(
                MemoryKind::Project,
                "repo",
                "Working convention: memory and RAG stay separate",
            )
            .expect("project memory"),
        )
        .expect("append project");
    store
        .append(
            MemoryRecord::new(
                MemoryKind::Task,
                "chat-session",
                "Turn summary should not project",
            )
            .expect("task memory")
            .with_tags(vec!["turn-summary".into()]),
        )
        .expect("append task");
    let old_relationship = store
        .append(
            MemoryRecord::new(
                MemoryKind::Relationship,
                "default",
                "User asked Ikaros to remember: commit whenever the implementation passes tests",
            )
            .expect("old relationship"),
        )
        .expect("append old relationship");

    let rendered = env.run(["memory", "projection", "render", "--scope", "repo"]);
    assert!(rendered.contains("projection rendered"));
    let journal = JsonlMemoryJournal::new(env.home.join("memory"));
    let entries = journal.list().expect("projection journal entries");
    assert!(entries.iter().any(
        |entry| entry.action == MemoryJournalAction::ProjectionRendered
            && entry.scope.as_deref() == Some("repo")
    ));
    let user_projection =
        fs::read_to_string(env.home.join("memory/projections/USER.md")).expect("user projection");
    let project_projection =
        fs::read_to_string(env.home.join("memory/projections/PROJECT.repo.md"))
            .expect("project projection");
    assert!(user_projection.contains("concise updates"));
    assert!(project_projection.contains("memory and RAG stay separate"));
    assert!(!user_projection.contains("Turn summary"));
    assert!(!project_projection.contains("Turn summary"));

    let shown = env.run(["memory", "projection", "show", "--scope", "repo"]);
    assert!(shown.contains("# User"));
    assert!(shown.contains("# Project Memory: repo"));

    let candidate_store = JsonlMemoryCandidateStore::new(env.home.join("memory"));
    let accepted = candidate_store
        .create(
            MemoryCandidate::new(
                MemoryKind::Relationship,
                "default",
                "User asked Ikaros to remember: do not commit unless explicitly requested",
                MemoryCandidateReason::ExplicitRemember,
                0.93,
            )
            .expect("accepted candidate"),
        )
        .expect("create accepted candidate");
    let rejected = candidate_store
        .create(
            MemoryCandidate::new(
                MemoryKind::Task,
                "chat-session",
                "Temporary PR scope: docs only",
                MemoryCandidateReason::TaskOutcome,
                0.45,
            )
            .expect("rejected candidate"),
        )
        .expect("create rejected candidate");

    let listed = env.run(["memory", "candidate", "list"]);
    assert!(listed.contains("\"status\": \"pending\""));
    assert!(listed.contains("do not commit"));
    let accepted_output = env.run([
        "memory",
        "candidate",
        "accept",
        &accepted.id,
        "--reason",
        "explicit user instruction",
        "--supersedes",
        &old_relationship.id,
    ]);
    assert!(accepted_output.contains("\"status\": \"accepted\""));
    let entries = journal.list().expect("candidate accept journal entries");
    assert!(entries.iter().any(
        |entry| entry.action == MemoryJournalAction::CandidateAccepted
            && entry.memory_id.as_deref() == Some(&accepted.id)
            && entry.scope.as_deref() == Some("default")
    ));
    assert!(
        entries
            .iter()
            .any(|entry| entry.action == MemoryJournalAction::Superseded
                && entry.memory_id.as_deref() == Some(&old_relationship.id)
                && entry.scope.as_deref() == Some("default"))
    );
    let user_projection =
        fs::read_to_string(env.home.join("memory/projections/USER.md")).expect("user projection");
    assert!(user_projection.contains("do not commit unless explicitly requested"));
    assert!(!user_projection.contains("commit whenever"));
    let records = store
        .list(ikaros_memory::MemoryQuery {
            scope: Some("default".into()),
            include_inactive: true,
            ..ikaros_memory::MemoryQuery::default()
        })
        .expect("memory records");
    assert!(records.iter().any(|record| {
        record.id == old_relationship.id && !record.active && record.superseded_by.is_some()
    }));
    let search = env.run([
        "memory",
        "search",
        "--kind",
        "relationship",
        "--scope",
        "default",
        "do not commit",
    ]);
    assert!(search.contains("do not commit unless explicitly requested"));
    let inactive_list_default = env.run([
        "memory",
        "list",
        "--kind",
        "relationship",
        "--scope",
        "default",
    ]);
    assert!(!inactive_list_default.contains("commit whenever"));
    let inactive_list_all = env.run([
        "memory",
        "list",
        "--kind",
        "relationship",
        "--scope",
        "default",
        "--include-inactive",
    ]);
    assert!(inactive_list_all.contains("commit whenever"));
    let inactive_default = env.run([
        "memory",
        "search",
        "--kind",
        "relationship",
        "--scope",
        "default",
        "commit whenever",
    ]);
    assert!(!inactive_default.contains("commit whenever"));
    let inactive_all = env.run([
        "memory",
        "search",
        "--kind",
        "relationship",
        "--scope",
        "default",
        "--include-inactive",
        "commit whenever",
    ]);
    assert!(inactive_all.contains("commit whenever"));
    let supersession_old = env.run(["memory", "supersession", &old_relationship.id]);
    assert!(supersession_old.contains("\"summary\": \"memory supersession explained\""));
    assert!(supersession_old.contains("\"status\": \"superseded\""));
    assert!(supersession_old.contains("\"replaced_by\""));
    assert!(supersession_old.contains("commit whenever"));
    assert!(supersession_old.contains("do not commit unless explicitly requested"));
    assert!(!supersession_old.contains("sk-secret"));
    let accepted_value: serde_json::Value =
        serde_json::from_str(&accepted_output).expect("accepted output json");
    let active_memory_id = accepted_value["memory_id"]
        .as_str()
        .expect("accepted memory id");
    let supersession_active = env.run(["memory", "supersession", active_memory_id]);
    assert!(supersession_active.contains("\"status\": \"active\""));
    assert!(supersession_active.contains("\"replaces\""));
    assert!(supersession_active.contains(&old_relationship.id));
    assert!(supersession_active.contains("do not commit unless explicitly requested"));
    assert!(!supersession_active.contains("sk-secret"));

    let rejected_output = env.run([
        "memory",
        "candidate",
        "reject",
        &rejected.id,
        "--reason",
        "temporary scope stays in episode history",
    ]);
    assert!(rejected_output.contains("\"status\": \"rejected\""));
    let entries = journal.list().expect("candidate reject journal entries");
    assert!(entries.iter().any(
        |entry| entry.action == MemoryJournalAction::CandidateRejected
            && entry.memory_id.as_deref() == Some(&rejected.id)
            && entry.scope.as_deref() == Some("chat-session")
    ));
    assert_eq!(
        candidate_store
            .list(ikaros_memory::MemoryCandidateQuery {
                status: Some(MemoryCandidateStatus::Pending),
                ..ikaros_memory::MemoryCandidateQuery::default()
            })
            .expect("pending candidates")
            .len(),
        0
    );
}

#[test]
fn memory_working_cli_lists_and_prunes_expired_scratchpad() {
    let env = TestHome::new();
    env.init();
    let store = JsonlWorkingMemoryStore::new(env.home.join("memory"));
    let mut expired = WorkingMemoryRecord::new(
        "session-1",
        MemoryKind::Task,
        "session-1",
        "expired temporary scope",
        None,
    )
    .expect("expired");
    expired.expires_at = Some("2000-01-01T00:00:00Z".into());
    let active = WorkingMemoryRecord::new(
        "session-1",
        MemoryKind::Task,
        "session-1",
        "active temporary scope",
        None,
    )
    .expect("active");
    store.append(expired).expect("append expired");
    store.append(active).expect("append active");

    let listed = env.run(["memory", "working", "list", "--session", "session-1"]);
    assert!(listed.contains("active temporary scope"));
    assert!(!listed.contains("expired temporary scope"));

    let pruned = env.run(["memory", "working", "prune"]);
    assert!(pruned.contains("\"summary\": \"working memory pruned\""));
    assert!(pruned.contains("\"expired_count\": 1"));
    assert!(pruned.contains("expired temporary scope"));

    let listed_after = env.run([
        "memory",
        "working",
        "list",
        "--session",
        "session-1",
        "--include-expired",
    ]);
    assert!(listed_after.contains("active temporary scope"));
    assert!(!listed_after.contains("expired temporary scope"));

    let journal = JsonlMemoryJournal::new(env.home.join("memory"));
    let entries = journal.list().expect("journal");
    assert!(
        entries
            .iter()
            .any(|entry| entry.action == MemoryJournalAction::WorkingMemoryExpired)
    );
}

#[test]
fn debug_context_and_memory_lifecycle_queries_session_timeline() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    fs::write(
        env.workspace.join("notes.md"),
        "alpha reference line\nbeta reference line\n",
    )
    .expect("write reference");

    let chat = env.run([
        "chat",
        "--message",
        "answer using @file:notes.md token=abc123",
        "--no-agent-loop",
    ]);
    let session_id = chat
        .lines()
        .find_map(|line| line.strip_prefix("chat_session: "))
        .expect("chat session id");

    let context = env.run(["debug", "context-diff", session_id]);
    let context_json: serde_json::Value = serde_json::from_str(&context).expect("context json");
    let turn_id = context_json["turns"]
        .as_array()
        .and_then(|turns| turns.first())
        .and_then(|turn| turn.as_str())
        .expect("context turn id")
        .to_owned();
    let expected_correlation_id = format!("session:{session_id}:turn:{turn_id}");
    assert!(context.contains("\"estimator\": \"mock-tokenizer-v1\""));
    assert!(context.contains("\"context_window\": 8192"));
    assert!(context.contains("\"context_compacted\": false"));
    assert!(
        context_json["prompt_stable_prefix_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("fnv1a64:"))
    );
    assert!(
        context_json["prompt_stable_prefix_message_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        context_json["prompt_stable_prefix_estimated_tokens"]
            .as_u64()
            .is_some_and(|tokens| tokens > 0)
    );
    assert_eq!(
        context_json["correlation_id"]
            .as_str()
            .expect("context correlation id"),
        expected_correlation_id
    );
    assert_eq!(
        context_json["correlations"][&turn_id]
            .as_str()
            .expect("context correlation map"),
        expected_correlation_id
    );
    assert!(context.contains("\"references\""));
    assert!(context.contains("@file:notes.md"));
    assert!(context.contains("\"sections\""));
    assert!(context.contains("\"prompt_sections\""));
    assert!(context.contains("\"kind\": \"references\""));
    assert!(context.contains("\"source\": \"context\""));
    assert!(!context.contains("abc123"));

    let memory = env.run(["debug", "memory-lifecycle", session_id]);
    let memory_json: serde_json::Value = serde_json::from_str(&memory).expect("memory json");
    assert!(memory.contains("\"phase\": \"turn_start\""));
    assert!(memory.contains("\"phase\": \"sync_turn\""));
    assert!(
        memory_json["memory_lifecycle_events"]
            .as_array()
            .expect("memory lifecycle events")
            .iter()
            .any(|event| event["correlation_id"].as_str() == Some(expected_correlation_id.as_str()))
    );
    assert!(
        memory_json["memory_journal_entries"]
            .as_array()
            .expect("memory journal entries")
            .iter()
            .any(|entry| entry["correlation_id"].as_str() == Some(expected_correlation_id.as_str()))
    );
    assert!(memory.contains("\"skipped\": true"));
    assert!(memory.contains("\"redaction_related\": true"));
    assert!(memory.contains("\"type\": \"session_turn\""));
    assert!(memory.contains("\"action\": \"skip\""));
    assert!(memory.contains("memory_journal.jsonl"));
    assert!(!memory.contains("abc123"));

    let missing = env.run_failure(["debug", "context-diff", "missing-session"]);
    assert!(missing.contains("session not found"));
    let missing_turn = env.run_failure([
        "debug",
        "context-diff",
        session_id,
        "--turn-id",
        "missing-turn",
    ]);
    assert!(missing_turn.contains("turn not found"));

    let memory_turn = env.run([
        "debug",
        "memory-lifecycle",
        session_id,
        "--turn-id",
        &turn_id,
    ]);
    let memory_turn_json: serde_json::Value =
        serde_json::from_str(&memory_turn).expect("memory turn json");
    assert_eq!(
        memory_turn_json["correlation_id"]
            .as_str()
            .expect("memory turn correlation id"),
        expected_correlation_id
    );
    assert!(memory_turn.contains("\"phase\": \"sync_turn\""));
    assert!(!memory_turn.contains("abc123"));
}

#[test]
fn debug_trace_exports_session_spans_without_prompt_or_secret_leak() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    fs::write(env.workspace.join("trace.md"), "trace reference\n").expect("trace reference");

    let chat = env.run([
        "chat",
        "--message",
        "trace this turn with @file:trace.md token=abc123",
        "--no-agent-loop",
    ]);
    let session_id = chat
        .lines()
        .find_map(|line| line.strip_prefix("chat_session: "))
        .expect("chat session id");

    let trace = env.run(["debug", "trace", session_id]);
    let trace_json: serde_json::Value = serde_json::from_str(&trace).expect("trace json");

    assert_eq!(trace_json["format"], "ikaros-trace-v1");
    assert_eq!(trace_json["session_id"], session_id);
    assert!(
        trace_json["turn_spans"]
            .as_array()
            .is_some_and(|spans| !spans.is_empty())
    );
    assert!(
        trace_json["event_counts"]["context"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        trace_json["event_counts"]["model"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        trace_json["ordered_events"]
            .as_array()
            .is_some_and(|events| events.iter().any(|event| event["category"] == "memory"))
    );
    assert!(!trace.contains("abc123"));
    assert!(!trace.contains("trace this turn"));

    let turn_id = trace_json["turn_spans"][0]["turn_id"]
        .as_str()
        .expect("turn id")
        .to_owned();
    let expected_correlation_id = format!("session:{session_id}:turn:{turn_id}");
    assert_eq!(
        trace_json["turn_spans"][0]["correlation_id"]
            .as_str()
            .expect("turn span correlation id"),
        expected_correlation_id
    );
    let ordered_events = trace_json["ordered_events"]
        .as_array()
        .expect("ordered events");
    assert!(
        ordered_events
            .iter()
            .any(|event| event["turn_id"].as_str() == Some(turn_id.as_str()))
    );
    for event in ordered_events
        .iter()
        .filter(|event| event["turn_id"].as_str() == Some(turn_id.as_str()))
    {
        assert_eq!(
            event["correlation_id"]
                .as_str()
                .expect("event correlation id"),
            expected_correlation_id
        );
    }

    let turn_trace = env.run(["debug", "trace", session_id, "--turn-id", &turn_id]);
    let turn_trace_json: serde_json::Value =
        serde_json::from_str(&turn_trace).expect("turn trace json");
    assert_eq!(
        turn_trace_json["turn_spans"]
            .as_array()
            .expect("turn spans array")
            .len(),
        1
    );
    assert_eq!(
        turn_trace_json["turn_spans"][0]["correlation_id"]
            .as_str()
            .expect("turn-filtered span correlation id"),
        expected_correlation_id
    );

    let missing = env.run_failure(["debug", "trace", session_id, "--turn-id", "missing-turn"]);
    assert!(missing.contains("turn not found"));
}

#[test]
fn debug_state_db_reports_sqlite_operational_status() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    env.run(["chat", "--message", "seed state db", "--no-agent-loop"]);

    let output = env.run(["debug", "state-db"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("state db json");

    assert_eq!(report["format"], "ikaros-state-db-v1");
    assert_eq!(report["journal_mode"], "wal");
    assert_eq!(report["integrity_check"]["ok"], true);
    assert_eq!(report["integrity_check"]["messages"][0], "ok");
    assert_eq!(
        report["write_policy"]["transaction_begin"],
        "BEGIN IMMEDIATE"
    );
    assert!(
        report["write_policy"]["busy_retry_attempts"]
            .as_u64()
            .is_some_and(|attempts| attempts > 0)
    );
    assert!(report["wal_checkpoint"]["log_frames"].is_number());
    assert!(
        report["search_indexes"]
            .as_array()
            .is_some_and(|indexes| indexes.iter().any(|index| {
                index["name"] == "session_entries_fts"
                    && index["index"] == "fts"
                    && index["available"] == true
            }))
    );
    assert!(
        report["search_indexes"]
            .as_array()
            .is_some_and(|indexes| indexes.iter().any(|index| {
                index["name"] == "session_entries_trigram"
                    && index["index"] == "trigram"
                    && index["available"] == true
            }))
    );
}

#[test]
fn debug_state_db_can_run_explicit_wal_checkpoint() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    env.run([
        "chat",
        "--message",
        "seed checkpoint state db",
        "--no-agent-loop",
    ]);

    let output = env.run(["debug", "state-db", "--checkpoint"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("state db json");

    assert_eq!(report["format"], "ikaros-state-db-v1");
    assert_eq!(report["checkpoint_performed"], true);
    assert!(report["wal_checkpoint"]["busy_frames"].is_number());
    assert!(report["wal_checkpoint"]["log_frames"].is_number());
    assert!(report["wal_checkpoint"]["checkpointed_frames"].is_number());
}

#[test]
fn debug_state_db_can_backup_and_vacuum_state_database() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    env.run([
        "chat",
        "--message",
        "seed backup state db",
        "--no-agent-loop",
    ]);
    let backup = env.home.join("backup/state-backup.db");
    let backup_arg = backup.to_string_lossy().to_string();

    let output = env.run(["debug", "state-db", "--backup", &backup_arg, "--vacuum"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("state db json");

    assert_eq!(report["format"], "ikaros-state-db-v1");
    assert_eq!(report["vacuum_performed"], true);
    assert_eq!(report["backup"]["created"], true);
    assert_eq!(report["backup"]["path"], backup_arg);
    assert!(backup.is_file());

    let backup_report = SqliteSessionStore::from_file(&backup)
        .operational_report()
        .expect("open backup");
    assert!(backup_report.integrity_check.ok);
}

#[test]
fn debug_state_db_can_write_repair_artifact() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    env.run([
        "chat",
        "--message",
        "seed repair state db",
        "--no-agent-loop",
    ]);
    let repair = env.home.join("repair/state-repair.db");
    let repair_arg = repair.to_string_lossy().to_string();

    let output = env.run(["debug", "state-db", "--repair", &repair_arg]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("state db json");

    assert_eq!(report["format"], "ikaros-state-db-v1");
    assert_eq!(report["repair"]["created"], true);
    assert_eq!(report["repair"]["path"], repair_arg);
    assert_eq!(report["repair"]["integrity_check"]["ok"], true);
    assert_eq!(report["repair"]["integrity_check"]["messages"][0], "ok");
    assert!(repair.is_file());

    let repair_report = SqliteSessionStore::from_file(&repair)
        .operational_report()
        .expect("open repair artifact");
    assert!(repair_report.integrity_check.ok);
}

#[test]
fn debug_state_db_can_restore_from_verified_backup_with_safety_copy() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    let first = env.run([
        "chat",
        "--message",
        "restore keeps this session",
        "--no-agent-loop",
    ]);
    let first_session = first
        .lines()
        .find_map(|line| line.strip_prefix("chat_session: "))
        .expect("first session");
    let backup = env.home.join("backup/state-restore-source.db");
    let backup_arg = backup.to_string_lossy().to_string();
    env.run(["debug", "state-db", "--backup", &backup_arg]);
    let second = env.run([
        "chat",
        "--message",
        "restore removes this later session",
        "--no-agent-loop",
    ]);
    let second_session = second
        .lines()
        .find_map(|line| line.strip_prefix("chat_session: "))
        .expect("second session");

    let output = env.run(["debug", "state-db", "--restore", &backup_arg]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("state db json");

    assert_eq!(report["format"], "ikaros-state-db-v1");
    assert_eq!(report["restore"]["restored"], true);
    assert_eq!(report["restore"]["source"], backup_arg);
    assert_eq!(report["restore"]["integrity_check"]["ok"], true);
    let safety_backup = report["restore"]["pre_restore_backup"]["path"]
        .as_str()
        .expect("safety backup path");
    assert!(
        fs::metadata(safety_backup)
            .expect("safety backup")
            .is_file()
    );

    let restored_store = SqliteSessionStore::from_file(
        report["state_db"]
            .as_str()
            .expect("state db path after restore"),
    );
    assert!(
        restored_store
            .get_session(&SessionId::from(first_session))
            .expect("first lookup")
            .is_some()
    );
    assert_eq!(
        restored_store
            .get_session(&SessionId::from(second_session))
            .expect("second lookup"),
        None
    );
}

#[test]
fn debug_state_db_can_prune_ended_sessions() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let state_output = env.run(["debug", "state-db"]);
    let state_report: serde_json::Value =
        serde_json::from_str(&state_output).expect("state db json");
    let state_db = state_report["state_db"]
        .as_str()
        .expect("state db path")
        .to_owned();
    let store = SqliteSessionStore::from_file(&state_db);
    let old_session_id = SessionId::from("cli-prune-old-session");
    let active_session_id = SessionId::from("cli-prune-active-session");
    let old_ended_at = time::OffsetDateTime::now_utc() - time::Duration::days(30);
    let cutoff = time::OffsetDateTime::now_utc() - time::Duration::days(7);
    let cutoff_arg = cutoff
        .format(&time::format_description::well_known::Rfc3339)
        .expect("format cutoff");

    store
        .upsert_session(&SessionRecord::new(
            old_session_id.clone(),
            SessionSource::Cli,
        ))
        .expect("old session");
    let mut old_entry = SessionEntry::new(old_session_id.clone(), SessionEntryKind::UserMessage);
    old_entry.visible_text = Some("old cli prune text".into());
    old_entry.payload = json!({"text": "old cli prune text"});
    store.append_entry(&old_entry).expect("old entry");
    store
        .finish_session(&old_session_id, old_ended_at)
        .expect("finish old session");
    store
        .upsert_session(&SessionRecord::new(
            active_session_id.clone(),
            SessionSource::Cli,
        ))
        .expect("active session");
    let mut active_entry =
        SessionEntry::new(active_session_id.clone(), SessionEntryKind::UserMessage);
    active_entry.visible_text = Some("active cli prune text".into());
    active_entry.payload = json!({"text": "active cli prune text"});
    store.append_entry(&active_entry).expect("active entry");

    let output = env.run(["debug", "state-db", "--prune-ended-before", &cutoff_arg]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("state db json");

    assert_eq!(report["format"], "ikaros-state-db-v1");
    assert_eq!(report["prune"]["ended_before"], cutoff_arg);
    assert_eq!(report["prune"]["sessions_pruned"], 1);
    assert_eq!(report["prune"]["entries_pruned"], 1);
    assert_eq!(report["integrity_check"]["ok"], true);
    assert_eq!(
        store.get_session(&old_session_id).expect("old lookup"),
        None
    );
    assert!(
        store
            .get_session(&active_session_id)
            .expect("active lookup")
            .is_some()
    );
}

#[test]
fn debug_logs_reports_redacted_paginated_audit_and_usage_entries() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    let audit = AuditLog::new(env.home.join("audit"));
    audit
        .append(
            AuditEvent::new(
                "policy_check",
                None,
                "allowed token=abc123",
                json!({"api_key": "sk-test-secret", "scope": "workspace"}),
            )
            .expect("audit event"),
        )
        .expect("append audit");
    audit
        .append(
            AuditEvent::new(
                "tool_call",
                None,
                "tool completed",
                json!({"tool": "fs_read"}),
            )
            .expect("audit event"),
        )
        .expect("append audit");
    ModelUsageLedger::new(env.home.join("audit"))
        .append(ModelUsageRecord {
            id: "usage-secret".into(),
            at: "2026-01-01T00:00:00Z".into(),
            provider: "openai-compatible".into(),
            model: "sk-raw-model-name".into(),
            prompt_tokens: Some(8),
            completion_tokens: Some(5),
            total_tokens: 13,
            cache_read_tokens: None,
            cache_write_tokens: None,
            estimated: false,
        })
        .expect("append usage");

    let output = env.run(["debug", "logs", "--page-size", "1"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("logs json");

    assert_eq!(report["format"], "ikaros-logs-v1");
    assert_eq!(report["counts"]["audit"], 2);
    assert_eq!(report["counts"]["model_usage"], 1);
    assert_eq!(report["pagination"]["page_size"], 1);
    assert_eq!(report["pagination"]["has_next"], true);
    assert_eq!(
        report["entries"].as_array().expect("entries array").len(),
        1
    );
    assert!(output.contains("[REDACTED_SECRET]"));
    assert!(!output.contains("abc123"));
    assert!(!output.contains("sk-test-secret"));
    assert!(!output.contains("sk-raw-model-name"));

    let usage_only = env.run(["debug", "logs", "--source", "model-usage"]);
    let usage_report: serde_json::Value =
        serde_json::from_str(&usage_only).expect("usage logs json");
    assert_eq!(usage_report["source"], "model_usage");
    assert_eq!(usage_report["counts"]["audit"], 0);
    assert_eq!(usage_report["counts"]["model_usage"], 1);
}

#[test]
fn debug_dump_writes_redacted_support_artifact() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    AuditLog::new(env.home.join("audit"))
        .append(
            AuditEvent::new(
                "support_event",
                None,
                "debug dump token=abc123",
                json!({"secret": "sk-dump-secret", "safe": "kept"}),
            )
            .expect("audit event"),
        )
        .expect("append audit");
    ModelUsageLedger::new(env.home.join("audit"))
        .append(ModelUsageRecord {
            id: "dump-usage".into(),
            at: "2026-01-02T00:00:00Z".into(),
            provider: "openai-compatible".into(),
            model: "sk-dump-model".into(),
            prompt_tokens: Some(3),
            completion_tokens: Some(4),
            total_tokens: 7,
            cache_read_tokens: None,
            cache_write_tokens: None,
            estimated: true,
        })
        .expect("append usage");
    let output_path = env.home.join("dumps/debug-dump.json");
    let output_arg = output_path.to_string_lossy().to_string();

    let output = env.run(["debug", "dump", "--output", &output_arg]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("dump json");

    assert_eq!(report["format"], "ikaros-debug-dump-v1");
    assert_eq!(report["redacted"], true);
    assert_eq!(report["export"]["created"], true);
    assert_eq!(report["export"]["path"], output_arg);
    assert_eq!(report["state_db"]["integrity_check"]["ok"], true);
    assert_eq!(report["logs"]["counts"]["audit"], 1);
    assert_eq!(report["logs"]["counts"]["model_usage"], 1);
    assert_eq!(report["sandbox"]["current"]["env_allowlist"], true);
    assert!(!output.contains("abc123"));
    assert!(!output.contains("sk-dump-secret"));
    assert!(!output.contains("sk-dump-model"));

    let artifact = fs::read_to_string(output_path).expect("debug dump artifact");
    let artifact_json: serde_json::Value =
        serde_json::from_str(&artifact).expect("debug dump artifact json");
    assert_eq!(artifact_json["format"], "ikaros-debug-dump-v1");
    assert_eq!(artifact_json["export"]["created"], true);
    assert_eq!(artifact_json["export"]["path"], output_arg);
    assert!(artifact.contains("[REDACTED_SECRET]"));
    assert!(!artifact.contains("abc123"));
    assert!(!artifact.contains("sk-dump-secret"));
    assert!(!artifact.contains("sk-dump-model"));
}

#[test]
fn debug_insights_reports_redacted_operational_summary() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    AuditLog::new(env.home.join("audit"))
        .append(
            AuditEvent::new(
                "insight_event",
                None,
                "insight audit token=abc123",
                json!({"api_key": "sk-insight-secret"}),
            )
            .expect("audit event"),
        )
        .expect("append audit");
    ModelUsageLedger::new(env.home.join("audit"))
        .append(ModelUsageRecord {
            id: "insight-usage".into(),
            at: "2026-01-03T00:00:00Z".into(),
            provider: "mock".into(),
            model: "sk-insight-model".into(),
            prompt_tokens: Some(6),
            completion_tokens: Some(7),
            total_tokens: 13,
            cache_read_tokens: Some(2),
            cache_write_tokens: Some(3),
            estimated: false,
        })
        .expect("append usage");
    let gateway = LocalGatewayStore::new(env.home.join("gateway"));
    gateway
        .enqueue(GatewayRoute::new(
            "cli",
            GatewayMessageKind::Chat,
            "gateway insight token=abc123",
            Some("local".into()),
        ))
        .expect("gateway enqueue");

    let output = env.run(["debug", "insights"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("insights json");

    assert_eq!(report["format"], "ikaros-debug-insights-v1");
    assert_eq!(report["config"]["valid"], true);
    assert_eq!(report["state_db"]["integrity_ok"], true);
    assert_eq!(report["logs"]["audit_count"], 1);
    assert_eq!(report["logs"]["model_usage_count"], 1);
    assert_eq!(report["logs"]["total_model_tokens"], 13);
    assert_eq!(report["logs"]["cache_read_tokens"], 2);
    assert_eq!(report["logs"]["cache_write_tokens"], 3);
    assert_eq!(report["providers"]["rows"][0]["kind"], "model");
    assert_eq!(report["providers"]["rows"][0]["live_smoke"], "offline");
    assert_eq!(report["gateway"]["pending"], 1);
    assert_eq!(report["status"], "attention");
    assert!(report["alerts"].as_array().is_some_and(|alerts| {
        alerts
            .iter()
            .any(|alert| alert["kind"] == "gateway_pending")
    }));
    assert!(output.contains("[REDACTED_SECRET]"));
    assert!(!output.contains("abc123"));
    assert!(!output.contains("sk-insight-secret"));
    assert!(!output.contains("sk-insight-model"));
}

#[test]
fn debug_sandbox_reports_isolation_matrix_and_current_limits() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();

    let output = env.run(["debug", "sandbox"]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("sandbox json");

    assert_eq!(report["format"], "ikaros-sandbox-v1");
    assert_eq!(report["current"]["level"], "network_restricted");
    assert_eq!(report["current"]["cwd_enforced"], true);
    assert_eq!(report["current"]["env_allowlist"], true);
    assert_eq!(report["current"]["timeout_capable"], true);
    let expected_timeout_strategy = if cfg!(windows) {
        "direct_child_kill"
    } else {
        "process_group_unix"
    };
    assert_eq!(
        report["current"]["process_timeout_strategy"],
        expected_timeout_strategy
    );
    assert_eq!(report["current"]["output_capable"], true);
    assert_eq!(report["current"]["file_write_scope"], "workspace_only");
    assert_eq!(report["current"]["network_egress"], "governed");
    assert_eq!(report["current"]["allow_provider_hosts"], true);
    assert_eq!(
        report["current"]["host_allowlist_mode"],
        "provider_hosts_plus_configured_hosts"
    );
    assert_eq!(report["current"]["restricted_ip_literal_block"], true);
    assert_eq!(report["current"]["dns_rebind_block"], true);
    assert_eq!(
        report["current"]["loopback_exception"],
        "explicit_loopback_hosts_only"
    );
    assert!(
        report["current"]["configured_allowed_host_count"]
            .as_u64()
            .is_some()
    );
    assert!(
        report["current"]["effective_allowed_host_count"]
            .as_u64()
            .is_some()
    );
    assert!(report["isolation_matrix"].as_array().is_some_and(|levels| {
        levels
            .iter()
            .any(|level| level["level"] == "container" && level["status"] == "available")
    }));
    assert!(report["isolation_matrix"].as_array().is_some_and(|levels| {
        levels
            .iter()
            .any(|level| level["level"] == "dry_run" && level["status"] == "available")
    }));
}

#[test]
fn debug_session_paginates_replay_and_exports_session_artifact() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    let chat = env.run([
        "chat",
        "--message",
        "debug session export one token=abc123",
        "--no-agent-loop",
    ]);
    let session_id = chat
        .lines()
        .find_map(|line| line.strip_prefix("chat_session: "))
        .expect("chat session id");
    env.run([
        "chat",
        "--message",
        "debug session export two",
        "--chat-session",
        session_id,
        "--no-agent-loop",
    ]);
    let export_path = env.home.join("exports/session-export.json");
    let export_arg = export_path.to_string_lossy().to_string();

    let output = env.run([
        "debug",
        "session",
        session_id,
        "--page-size",
        "1",
        "--export",
        &export_arg,
    ]);
    let report: serde_json::Value = serde_json::from_str(&output).expect("session debug json");

    assert_eq!(report["format"], "ikaros-session-debug-v1");
    assert_eq!(report["session_id"], session_id);
    assert_eq!(report["pagination"]["page_size"], 1);
    assert_eq!(report["pagination"]["entries"]["has_next"], true);
    assert_eq!(
        report["entries"].as_array().expect("paged entries").len(),
        1
    );
    assert_eq!(
        report["agent_events"]
            .as_array()
            .expect("paged agent events")
            .len(),
        1
    );
    assert!(report["counts"]["agent_events"].as_u64().unwrap_or(0) > 1);
    assert_eq!(report["export"]["created"], true);
    assert_eq!(report["export"]["path"], export_arg);
    assert!(export_path.is_file());
    let report_turn_id = report["agent_events"][0]["turn_id"]
        .as_str()
        .expect("paged agent event turn id");
    let expected_correlation_id = format!("session:{session_id}:turn:{report_turn_id}");
    assert_eq!(
        report["turn_correlations"][report_turn_id]
            .as_str()
            .expect("session debug turn correlation"),
        expected_correlation_id
    );

    let artifact_text = fs::read_to_string(&export_path).expect("export file");
    assert!(!artifact_text.contains("abc123"));
    let artifact: serde_json::Value =
        serde_json::from_str(&artifact_text).expect("session export json");
    assert_eq!(artifact["format"], "ikaros-session-export-v1");
    assert_eq!(artifact["redacted"], true);
    assert_eq!(artifact["session"]["session_id"], session_id);
    assert!(
        artifact["entries"]
            .as_array()
            .is_some_and(|entries| entries.len() >= 2)
    );
    assert!(
        artifact["agent_events"]
            .as_array()
            .is_some_and(|events| !events.is_empty())
    );
    assert_eq!(
        artifact["turn_correlations"][report_turn_id]
            .as_str()
            .expect("export turn correlation"),
        expected_correlation_id
    );
}

#[test]
fn debug_context_diff_explains_compacted_protected_reference_context() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    let state_dir = env.home.join("agents/debug");
    let store = SqliteSessionStore::new(&state_dir);
    let session_id = SessionId::from("debug-context-session");
    let turn_id = TurnId::from("debug-turn");
    store
        .upsert_session(&SessionRecord::new(session_id.clone(), SessionSource::Cli))
        .expect("session");
    store
        .append_agent_event(&AgentEvent::new(
            session_id.clone(),
            turn_id.clone(),
            None,
            AgentEventSource::Context,
            AgentEventKind::ContextDiff,
            json!({
                "budget": {
                    "estimator": "fixture-estimator-v1",
                    "context_window": 128,
                    "max_tokens": 64,
                    "used_tokens": 50
                },
                "prompt_sections": [
                    {
                        "kind": "references",
                        "title": "References",
                        "source": "context",
                        "priority": 90,
                        "estimated_tokens": 10,
                        "redaction": "redacted",
                        "content": "full prompt content must stay hidden"
                    }
                ],
                "prompt_stable_prefix_hash": "fnv1a64:1234567890abcdef",
                "prompt_stable_prefix_message_count": 1,
                "prompt_stable_prefix_estimated_tokens": 12,
                "sections": [
                    {
                        "kind": "references",
                        "label": "References",
                        "estimated_tokens": 10,
                        "protected": true,
                        "lines": ["@file:src/lib.rs:1-2"]
                    },
                    {
                        "kind": "history",
                        "label": "History",
                        "estimated_tokens": 40,
                        "protected": false
                    }
                ],
                "diff": {
                    "added": [
                        {
                            "section": "references",
                            "tokens": 10,
                            "preview": "@file:src/lib.rs:1-2"
                        }
                    ],
                    "removed": [
                        {
                            "section": "rag",
                            "tokens": 5,
                            "preview": "stale retrieval"
                        }
                    ],
                    "compressed": [
                        {
                            "section": "history",
                            "tokens": 20,
                            "preview": "older history detail"
                        }
                    ]
                },
                "references": [
                    {
                        "reference": {
                            "raw": "@file:src/lib.rs:1-2",
                            "kind": {
                                "type": "file",
                                "data": {
                                    "path": "src/lib.rs",
                                    "start_line": 1,
                                    "end_line": 2
                                }
                            }
                        },
                        "line": "fn example()"
                    }
                ],
                "compressed_sections": [
                    {
                        "section": "history",
                        "original_tokens": 40,
                        "kept_tokens": 20,
                        "omitted_tokens": 20,
                        "omitted_lines": 3
                    }
                ],
                "compression_summary": "history: omitted 3 line(s), about 20 tokens",
                "continuation_prompt": "Some local context was compacted; do not invent omitted details.",
                "protected_sections": ["references"]
            }),
        ))
        .expect("context diff event");
    store
        .append_agent_event(&AgentEvent::new(
            session_id.clone(),
            turn_id.clone(),
            None,
            AgentEventSource::Context,
            AgentEventKind::ContextCompacted,
            json!({
                "summary": "history: omitted 3 line(s), about 20 tokens",
                "continuation_prompt": "Some local context was compacted; do not invent omitted details.",
                "compressed_sections": [
                    {
                        "section": "history",
                        "omitted_tokens": 20,
                        "omitted_lines": 3
                    }
                ]
            }),
        ))
        .expect("compaction event");

    let output = env.run([
        "debug",
        "context-diff",
        "debug-context-session",
        "--turn-id",
        "debug-turn",
    ]);
    assert!(output.contains("\"estimator\": \"fixture-estimator-v1\""));
    assert!(output.contains("\"context_window\": 128"));
    assert!(output.contains("\"context_compacted\": true"));
    assert!(output.contains("\"prompt_stable_prefix_hash\""));
    assert!(output.contains("fnv1a64:1234567890abcdef"));
    assert!(output.contains("\"prompt_stable_prefix_message_count\": 1"));
    assert!(output.contains("\"prompt_stable_prefix_estimated_tokens\": 12"));
    assert!(output.contains("\"compressed\""));
    assert!(output.contains("\"protected\": true"));
    assert!(output.contains("\"protected_sections\""));
    assert!(output.contains("@file:src/lib.rs:1-2"));
    assert!(output.contains("history: omitted 3 line(s), about 20 tokens"));
    assert!(output.contains("do not invent omitted details"));
    assert!(output.contains("\"prompt_sections\""));
    assert!(!output.contains("full prompt content must stay hidden"));
    assert!(
        !output.contains("\"content\""),
        "debug context-diff must expose prompt section metadata only"
    );
}

#[test]
fn debug_memory_lifecycle_reports_runtime_policy_actions() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

memory:
  backend: jsonl
  policy:
    promote_threshold: 0.70
    demote_threshold: 0.45
    forget_threshold: 0.30
    max_records_per_scope: 2

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
    .expect("policy config");
    let store = JsonlMemoryStore::new(env.home.join("memory"));
    let mut promote = MemoryRecord::new(
        MemoryKind::Task,
        "policy-session",
        "remember stable project preference concise updates",
    )
    .expect("promote");
    promote.created_at = "2099-01-01T00:00:00Z".into();
    promote.confidence = Some(1.0);
    promote.source = Some("manual".into());
    promote.tags = vec!["turn-summary".into(), "memory-lifecycle".into()];
    store.append(promote).expect("append promote");

    let mut demote =
        MemoryRecord::new(MemoryKind::Task, "policy-session", "stale task note").expect("demote");
    demote.created_at = "2002-01-01T00:00:00Z".into();
    demote.confidence = Some(0.2);
    store.append(demote).expect("append demote");

    let mut quota = MemoryRecord::new(MemoryKind::Task, "policy-session", "low confidence task")
        .expect("quota");
    quota.created_at = "2001-01-01T00:00:00Z".into();
    quota.confidence = Some(0.3);
    store.append(quota).expect("append quota");

    let mut forget = MemoryRecord::new(
        MemoryKind::Task,
        "policy-session",
        "obsolete temporary note",
    )
    .expect("forget");
    forget.created_at = "2000-01-01T00:00:00Z".into();
    forget.confidence = Some(0.0);
    store.append(forget).expect("append forget");

    let chat = env.run([
        "chat",
        "--chat-session",
        "policy-session",
        "--message",
        "remember that I prefer concise updates",
        "--no-agent-loop",
    ]);
    assert!(chat.contains("chat_session: policy-session"));

    let memory = env.run(["debug", "memory-lifecycle", "policy-session"]);
    assert!(memory.contains("\"action\": \"append\""));
    assert!(memory.contains("\"action\": \"promote\""));
    assert!(memory.contains("\"action\": \"demote\""));
    assert!(memory.contains("\"action\": \"forget\""));
    assert!(memory.contains("quota removed lower score memory"));
    assert!(memory.contains("\"memory_journal_action_counts\""));
    assert!(memory.contains("\"source_ref\""));
    assert!(!memory.contains("abc123"));
}

#[test]
fn debug_continuations_reports_queue_status_and_redacts_payload() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    let state_dir = env.home.join("agents/debug");
    let store = SqliteSessionStore::new(&state_dir);
    let session_id = SessionId::from("debug-continuation-session");
    let turn_id = TurnId::from("debug-continuation-turn");
    store
        .upsert_session(&SessionRecord::new(session_id.clone(), SessionSource::Cli))
        .expect("session");
    store
        .append_agent_event(&AgentEvent::new(
            session_id.clone(),
            TurnId::from("turn-without-continuations"),
            None,
            AgentEventSource::Runtime,
            AgentEventKind::TurnStart,
            json!({"fixture": "no continuation turn"}),
        ))
        .expect("turn without continuations");

    let mut queued =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::NextTurn);
    queued.turn_id = Some(turn_id.clone());
    queued.payload = json!({"content": "continue safely", "api_key": "sk-continuation-secret"});
    store.enqueue_continuation(&queued).expect("queued");

    let mut running =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::FollowUp);
    running.turn_id = Some(turn_id.clone());
    running.payload = json!({"content": "running"});
    store.enqueue_continuation(&running).expect("running");
    store
        .claim_next_continuation(
            &SessionContinuationClaim::for_session(session_id.clone())
                .with_kinds([SessionContinuationKind::FollowUp])
                .with_lease_owner("debug-worker")
                .with_lease_duration_seconds(60),
        )
        .expect("claim running")
        .expect("running claimed");

    let mut expired =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::Steer);
    expired.turn_id = Some(turn_id.clone());
    expired.payload = json!({"content": "lease reclaim"});
    store.enqueue_continuation(&expired).expect("expired");
    store
        .claim_next_continuation(
            &SessionContinuationClaim::for_session(session_id.clone())
                .with_kinds([SessionContinuationKind::Steer])
                .with_lease_owner("expired-worker")
                .with_lease_duration_seconds(0),
        )
        .expect("claim expired")
        .expect("expired claimed");
    store
        .claim_next_continuation(
            &SessionContinuationClaim::for_session(session_id.clone())
                .with_kinds([SessionContinuationKind::Steer])
                .with_lease_owner("replacement-worker")
                .with_lease_duration_seconds(60),
        )
        .expect("reclaim expired")
        .expect("expired reclaimed");

    let mut failed =
        SessionContinuationInput::new(session_id.clone(), SessionContinuationKind::Retry);
    failed.turn_id = Some(turn_id.clone());
    let failed = store.enqueue_continuation(&failed).expect("failed");
    let failed_claim = store
        .claim_next_continuation(
            &SessionContinuationClaim::for_session(session_id.clone())
                .with_kinds([SessionContinuationKind::Retry])
                .with_lease_owner("retry-worker"),
        )
        .expect("claim failed")
        .expect("failed claimed");
    assert_eq!(failed_claim.continuation_id, failed.continuation_id);
    store
        .fail_continuation(&failed.continuation_id, "provider unavailable")
        .expect("fail");

    let cancelled = store
        .enqueue_continuation(&SessionContinuationInput::new(
            session_id.clone(),
            SessionContinuationKind::Compact,
        ))
        .expect("cancelled");
    store
        .cancel_continuation(&cancelled.continuation_id, "operator cancelled")
        .expect("cancel");

    let output = env.run([
        "debug",
        "continuations",
        "debug-continuation-session",
        "--turn-id",
        "debug-continuation-turn",
    ]);
    assert!(output.contains("\"queued\": 1"));
    assert!(output.contains("\"running\": 2"));
    assert!(output.contains("\"failed\": 1"));
    assert!(!output.contains("\"cancelled\": 1"));
    assert!(output.contains("\"lease_owner\": \"debug-worker\""));
    assert!(output.contains("\"lease_owner\": \"replacement-worker\""));
    assert!(output.contains("\"attempt_count\": 1"));
    assert!(output.contains("\"attempt_count\": 2"));
    assert!(output.contains("\"error\": \"provider unavailable\""));
    assert!(output.contains("\"error\": \"lease expired\""));
    assert!(output.contains("\"lease_expired\": false"));
    assert!(output.contains("\"status_reason\": \"lease_expired\""));
    assert!(output.contains("\"reason\": \"lease_expired\""));
    assert!(output.contains("\"kind\": \"worker_lease\""));
    assert!(output.contains("\"status_reason\": \"failed\""));
    assert!(output.contains("\"terminal\""));
    assert!(output.contains("\"reason\": \"failed\""));
    assert!(output.contains("\"ended_at\""));
    assert!(output.contains("[REDACTED_SECRET]"));
    assert!(!output.contains("sk-continuation-secret"));
    let output_json: serde_json::Value =
        serde_json::from_str(&output).expect("continuation debug json");
    let expected_correlation_id = "session:debug-continuation-session:turn:debug-continuation-turn";
    for continuation in output_json["continuations"]
        .as_array()
        .expect("continuation array")
    {
        assert_eq!(
            continuation["correlation_id"]
                .as_str()
                .expect("continuation correlation id"),
            expected_correlation_id
        );
    }

    let all = env.run(["debug", "continuations", "debug-continuation-session"]);
    assert!(all.contains("\"cancelled\": 1"));
    assert!(all.contains("\"operator cancelled\""));

    let empty_turn = env.run([
        "debug",
        "continuations",
        "debug-continuation-session",
        "--turn-id",
        "turn-without-continuations",
    ]);
    assert!(empty_turn.contains("\"continuation_count\": 0"));
    assert!(!empty_turn.contains("turn not found"));

    let missing_turn = env.run_failure([
        "debug",
        "continuations",
        "debug-continuation-session",
        "--turn-id",
        "missing-turn",
    ]);
    assert!(missing_turn.contains("turn not found"));
    let missing_session = env.run_failure(["debug", "continuations", "missing-session"]);
    assert!(missing_session.contains("session not found"));
}

#[test]
fn memory_provider_commands_reject_enabled_external_descriptors() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"schema_version: 1

model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  backend: jsonl
  embedding_provider: hash
  embedding_model: text-embedding-3-small

voice:
  tts:
    provider: mock
    model: mock-tts
  asr:
    provider: mock
    model: mock-asr

memory:
  backend: jsonl
  external_providers:
    - id: remote-a
      provider: plugin
      enabled: true
      endpoint: http://127.0.0.1:8787
      api_key: memory-key-a
    - id: remote-b
      provider: plugin
      enabled: true
      endpoint: http://127.0.0.1:8788
      api_key: memory-key-b
"#,
    )
    .expect("provider config");

    let providers = env.run_failure(["memory", "provider", "list"]);
    assert!(providers.contains("configuration validation failed"));
    assert!(providers.contains("memory.external_providers"));
    assert!(providers.contains("external memory providers are descriptors only"));

    let doctor = env.run(["doctor"]);
    assert!(doctor.contains("memory_providers:"));
    assert!(doctor.contains("issues=1"));
    assert!(
        doctor.contains("memory_provider_issue: only one external memory provider may be active")
    );
}

#[test]
fn debug_trace_surfaces_model_diagnostic_kinds_from_session_timeline() {
    let env = TestHome::new();
    env.init();
    env.use_offline_mock_config();
    let state_dir = env.home.join("agents/debug-diag");
    let store = SqliteSessionStore::new(&state_dir);
    let session_id = SessionId::from("debug-diag-session");
    let turn_id = TurnId::from("debug-diag-turn");
    store
        .upsert_session(&SessionRecord::new(session_id.clone(), SessionSource::Cli))
        .expect("session");

    let start = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Runtime,
        AgentEventKind::TurnStart,
        json!({}),
    );
    store.append_agent_event(&start).expect("start");

    let retry_diag = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        Some(start.event_id.clone()),
        AgentEventSource::Model,
        AgentEventKind::ModelDiagnostic(ikaros_models::ModelRequestDiagnostic {
            kind: "provider_retry_failed".into(),
            message: "provider openai-compatible/kimi-k2.6 retry attempt 1 failed with rate_limit error token=abc123".into(),
            parameter: None,
        }),
        json!({"diagnostic_kind": "provider_retry_failed"}),
    );
    store.append_agent_event(&retry_diag).expect("retry diag");

    let fallback_diag = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        Some(retry_diag.event_id.clone()),
        AgentEventSource::Model,
        AgentEventKind::ModelDiagnostic(ikaros_models::ModelRequestDiagnostic {
            kind: "fallback_provider_selected".into(),
            message: "provider openai-compatible/qwen-2.5-72b selected after 1 fallback attempt(s)"
                .into(),
            parameter: None,
        }),
        json!({"diagnostic_kind": "fallback_provider_selected"}),
    );
    store
        .append_agent_event(&fallback_diag)
        .expect("fallback diag");

    let end = AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        Some(fallback_diag.event_id.clone()),
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({}),
    );
    store.append_agent_event(&end).expect("end");

    let trace = env.run([
        "debug",
        "trace",
        session_id.as_str(),
        "--turn-id",
        turn_id.as_str(),
    ]);
    assert!(trace.contains("\"diagnostic_kind\": \"provider_retry_failed\""));
    assert!(trace.contains("\"diagnostic_kind\": \"fallback_provider_selected\""));
    assert!(trace.contains("\"category\": \"model\""));
    assert!(trace.contains("\"kind\": \"model_diagnostic\""));
    assert!(trace.contains("provider_retry_failed"));
    assert!(trace.contains("fallback_provider_selected"));
    assert!(
        !trace.contains("abc123"),
        "secret must be redacted in trace output"
    );
}
