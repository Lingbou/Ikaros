// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use crate::support::TestHome;
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSource, SessionId, SessionRecord, SessionSource,
    SessionStore, SqliteSessionStore, TurnId,
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
    assert!(context.contains("\"estimator\": \"mock-tokenizer-v1\""));
    assert!(context.contains("\"context_window\": 8192"));
    assert!(context.contains("\"context_compacted\": false"));
    assert!(context.contains("\"references\""));
    assert!(context.contains("@file:notes.md"));
    assert!(context.contains("\"sections\""));
    assert!(!context.contains("abc123"));

    let memory = env.run(["debug", "memory-lifecycle", session_id]);
    assert!(memory.contains("\"phase\": \"turn_start\""));
    assert!(memory.contains("\"phase\": \"sync_turn\""));
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
    assert!(memory_turn.contains("\"phase\": \"sync_turn\""));
    assert!(!memory_turn.contains("abc123"));
}

#[test]
fn debug_context_diff_explains_compacted_protected_reference_context() {
    let env = TestHome::new();
    env.init();
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
    assert!(output.contains("\"compressed\""));
    assert!(output.contains("\"protected\": true"));
    assert!(output.contains("\"protected_sections\""));
    assert!(output.contains("@file:src/lib.rs:1-2"));
    assert!(output.contains("history: omitted 3 line(s), about 20 tokens"));
    assert!(output.contains("do not invent omitted details"));
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
