// SPDX-License-Identifier: GPL-3.0-only

use std::fs;

use crate::support::TestHome;
use ikaros_memory::{
    JsonlMemoryCandidateStore, JsonlMemoryJournal, JsonlMemoryStore, JsonlWorkingMemoryStore,
    MemoryCandidate, MemoryCandidateReason, MemoryCandidateStatus, MemoryJournal,
    MemoryJournalAction, MemoryKind, MemoryRecord, MemoryStore, WorkingMemoryRecord,
};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSource, SessionContinuationClaim,
    SessionContinuationInput, SessionContinuationKind, SessionId, SessionRecord, SessionSource,
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
fn memory_cli_filters_observer_subject_perspective() {
    let env = TestHome::new();
    env.init();

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
fn debug_memory_lifecycle_reports_runtime_policy_actions() {
    let env = TestHome::new();
    env.init();
    fs::write(
        env.home.join("config.yaml"),
        r#"model:
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
