// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use crate::support::TestHome;

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
        "--context-char-budget",
        "64",
    ]);
    assert!(chat.contains("ok: true"));
    assert!(chat.contains("provider: mock"));
    assert!(chat.contains("emotion: Satisfied"));
    assert!(chat.contains("model_usage:"));
    assert!(chat.contains("chat_history:"));
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
    assert!(chat_next.contains("context: relationship=0 history=1 memory=0 rag=0"));

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
    let history_file = fs::read_to_string(env.home.join("chat/history.jsonl")).expect("history");
    assert!(history_file.contains("[REDACTED_SECRET]"));
    assert!(!history_file.contains("abc123"));

    let deleted_session = env.run(["chat", "--history-delete-session", session_id]);
    assert!(deleted_session.contains(&format!("deleted_session: {session_id}")));
    assert!(deleted_session.contains("deleted_records: 2"));
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
fn local_memory_and_message_gateway_smoke_paths_run_offline() {
    let env = TestHome::new();
    env.init();

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
fn memory_provider_inspection_reports_external_single_active_issues() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"memory:
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

    let providers = env.run(["memory", "provider", "list"]);
    assert!(providers.contains("\"id\": \"remote-a\""));
    assert!(providers.contains("\"state\": \"blocked\""));
    assert!(providers.contains("only one external memory provider"));

    let doctor = env.run(["doctor"]);
    assert!(doctor.contains("memory_provider_issue: only one external memory provider"));
}
