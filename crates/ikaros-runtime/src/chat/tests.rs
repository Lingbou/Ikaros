// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use async_trait::async_trait;
use ikaros_core::{
    AgentProfile, ContextBuilder, IkarosError, IkarosPaths, RagConfig, ResolvedAgentProfile, Result,
};
use ikaros_models::{ModelProvider, ModelRequest, ModelResponse, ModelStreamEvent};
use ikaros_session::{
    AgentEventKind, PersistingAgentTurnSink, SessionEntryKind, SessionId, SessionSource,
    SessionStore, SqliteSessionStore,
};
use ikaros_soul::{EmotionState, PersonaLoader};
use std::{fs, sync::Arc};

#[derive(Debug)]
struct FailingProvider;

#[async_trait]
impl ModelProvider for FailingProvider {
    fn name(&self) -> &str {
        "failing"
    }

    async fn generate(&self, _request: ModelRequest) -> Result<ModelResponse> {
        Err(IkarosError::Message(
            "provider failed with token=abc123".into(),
        ))
    }
}

#[test]
fn chat_context_extractors_redact_values() {
    let memory = serde_json::json!([
        {
            "kind": "Relationship",
            "scope": "user",
            "content": "Relationship context should be handled separately"
        },
        {
            "kind": "Project",
            "scope": "repo",
            "content": "Prefer local RAG and never expose token=abc123"
        }
    ]);
    let memory_context = extract_memory_context(&memory, 5);
    assert_eq!(memory_context.len(), 1);
    assert!(memory_context[0].contains("[Project/repo]"));
    assert!(!memory_context[0].contains("Relationship context"));
    assert!(!memory_context[0].contains("abc123"));
    assert!(memory_context[0].contains("[REDACTED_SECRET]"));

    let rag = serde_json::json!([
        {
            "chunk": {"content": "RAG context with sk-not-real"},
            "citation": {"source_path": "docs/rag.md", "line_start": 3, "line_end": 7}
        }
    ]);
    let rag_context = extract_rag_context(&rag, 5);
    assert_eq!(rag_context.len(), 1);
    assert!(rag_context[0].contains("docs/rag.md:3-7"));
    assert!(!rag_context[0].contains("sk-not-real"));
}

#[test]
fn chat_system_prompt_uses_context_and_redacts() {
    let context = ContextBuilder::new()
        .persona_context("Persona token=abc123")
        .relationship_context(vec!["Relationship prefers concise updates".into()])
        .reference_context(vec!["Reference src/lib.rs".into()])
        .chat_history_context(vec!["Earlier user asked for a quiet status".into()])
        .memory_context(vec!["Memory sk-not-real".into()])
        .rag_context(vec!["RAG safe citation".into()])
        .build();
    let prompt = render_chat_system_prompt(&context);
    assert!(prompt.contains("Local relationship context"));
    assert!(prompt.contains("Relationship prefers concise updates"));
    assert!(prompt.contains("Local reference context"));
    assert!(prompt.contains("Reference src/lib.rs"));
    assert!(prompt.contains("Local chat history context"));
    assert!(prompt.contains("Earlier user asked for a quiet status"));
    assert!(prompt.contains("Local memory context"));
    assert!(prompt.contains("Local RAG context"));
    assert!(prompt.contains("RAG safe citation"));
    assert!(!prompt.contains("abc123"));
    assert!(!prompt.contains("sk-not-real"));
    assert!(prompt.contains("[REDACTED_SECRET]"));
}

#[test]
fn persona_agent_context_includes_profile_overlay_and_redacts() {
    let persona = PersonaLoader::parse(PersonaLoader::default_markdown()).expect("default persona");
    let mut profile = AgentProfile::plan();
    profile.persona_overlay.push_str(" token=abc123");
    let agent = ResolvedAgentProfile {
        name: "plan".into(),
        profile,
    };
    let context = render_persona_agent_context(&persona, &agent);
    assert!(context.contains("Agent profile: plan (plan)"));
    assert!(context.contains("read-only planning mode"));
    assert!(context.contains("[REDACTED_SECRET]"));
    assert!(!context.contains("abc123"));
}

#[test]
fn chat_context_budget_preserves_priority_and_truncates() {
    let context = ChatContext {
        relationship: vec!["rel".into()],
        references: vec!["ref".into()],
        history: vec!["hist".into()],
        memory: vec!["memory-context-is-long-enough-to-truncate".into()],
        rag: vec!["rag should be omitted".into()],
    };

    let budgeted = super::context::apply_chat_context_token_budget(context, 12);

    assert_eq!(budgeted.relationship, vec!["rel"]);
    assert_eq!(budgeted.references, vec!["ref"]);
    assert_eq!(budgeted.history, vec!["hist"]);
    assert_eq!(budgeted.memory.len(), 1);
    assert!(budgeted.memory[0].contains("[truncated]"));
    assert!(budgeted.rag.is_empty());
    assert!(super::context::chat_context_token_count_with_default(&budgeted) <= 12);
}

#[test]
fn chat_context_budget_zero_keeps_context_unbounded() {
    let context = ChatContext {
        relationship: vec!["rel".into()],
        references: vec!["ref".into()],
        history: vec!["hist".into()],
        memory: vec!["memory".into()],
        rag: vec!["rag".into()],
    };

    assert_eq!(
        super::context::apply_chat_context_token_budget(context.clone(), 0),
        context
    );
}

#[test]
fn chat_history_context_summarizes_older_turns_for_all_backends() {
    for backend in ["jsonl", "sqlite"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = ChatHistoryStore::new_with_backend(temp.path(), backend).expect("store");
        store
            .append(&chat_history_record(
                "session-a",
                "first long-running topic",
            ))
            .expect("append first");
        store
            .append(&chat_history_record("session-a", "second continuity point"))
            .expect("append second");
        store
            .append(&chat_history_record("session-a", "third recent turn"))
            .expect("append third");

        let lines = store.context_lines(1, 4).expect("context lines");

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("older chat summary turns=2"));
        assert!(lines[0].contains("first long-running topic"));
        assert!(lines[0].contains("second continuity point"));
        assert!(lines[1].contains("third recent turn"));
    }
}

#[test]
fn chat_history_context_can_be_scoped_to_session_for_all_backends() {
    for backend in ["jsonl", "sqlite"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = ChatHistoryStore::new_with_backend(temp.path(), backend).expect("store");
        store
            .append(&chat_history_record("session-a", "alpha first"))
            .expect("append a1");
        store
            .append(&chat_history_record("session-b", "beta should stay out"))
            .expect("append b");
        store
            .append(&chat_history_record("session-a", "alpha recent"))
            .expect("append a2");

        let lines = store
            .context_lines_for_session("session-a", 1, 4)
            .expect("session context lines");

        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("older chat summary turns=1"));
        assert!(lines[0].contains("alpha first"));
        assert!(lines[1].contains("alpha recent"));
        assert!(lines.iter().all(|line| !line.contains("beta")));
    }
}

#[test]
fn chat_history_session_summaries_group_recent_sessions_for_all_backends() {
    for backend in ["jsonl", "sqlite"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = ChatHistoryStore::new_with_backend(temp.path(), backend).expect("store");
        store
            .append(&chat_history_record("session-a", "alpha first"))
            .expect("append a1");
        store
            .append(&chat_history_record("session-b", "beta only"))
            .expect("append b");
        store
            .append(&chat_history_record(
                "session-a",
                "alpha latest token=abc123",
            ))
            .expect("append a2");

        let summaries = store.session_summaries(10).expect("session summaries");

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].session_id, "session-a");
        assert_eq!(summaries[0].turns, 2);
        assert_eq!(summaries[0].agents, vec!["build"]);
        assert!(summaries[0].last_user_message.contains("[REDACTED_SECRET]"));
        assert!(!summaries[0].last_user_message.contains("abc123"));
        assert_eq!(summaries[1].session_id, "session-b");
        assert_eq!(summaries[1].turns, 1);

        let limited = store.session_summaries(1).expect("limited summaries");
        assert_eq!(limited.len(), 1);
        assert_eq!(limited[0].session_id, "session-a");
        assert!(store.session_summaries(0).expect("zero limit").is_empty());
    }
}

#[test]
fn relationship_learning_extracts_clear_preferences_and_redacts() {
    let candidates = super::learning::extract_relationship_memory_candidates(
        "I prefer concise updates. Call me Ling. remember token=abc123",
    );

    assert_eq!(
        candidates,
        vec![
            "User preference: concise updates",
            "User preferred name: Ling"
        ]
    );
    assert!(
        super::learning::extract_relationship_memory_candidates(
            "我喜欢安静一点的回复。请记住我偏好本地优先。"
        )
        .iter()
        .any(|candidate| candidate.contains("User preference: 安静一点的回复"))
    );
    assert!(super::learning::extract_relationship_memory_candidates("hello world").is_empty());
}

#[test]
fn cloud_rag_is_not_used_for_redacted_safe_read_chat_lookup() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let registry = ikaros_skills::builtin_registry(ikaros_skills::SkillEnvironment {
        workspace_root: workspace,
        memory_store: ikaros_memory::LocalMemoryStore::new(temp.path().join("memory"), "jsonl")
            .expect("memory"),
        rag_index: ikaros_rag::LocalRagStore::new(temp.path().join("rag"), "jsonl").expect("rag"),
        rag_config: RagConfig {
            embedding_provider: "openai-compatible".into(),
            ..RagConfig::default()
        },
        rag_provider: ikaros_core::RemoteProviderConfig::default(),
        persona_path: temp.path().join("persona.md"),
        skills_dir: temp.path().join("skills"),
        voice_tts: ikaros_voice::VoiceProviderConfig::mock_tts(),
        voice_tts_provider: ikaros_core::RemoteProviderConfig::default(),
        voice_asr: ikaros_voice::VoiceProviderConfig::mock_asr(),
        voice_asr_provider: ikaros_core::RemoteProviderConfig::default(),
    });

    assert!(!context_lookup_is_safe_read(&registry, "rag_search"));
}

#[tokio::test]
async fn run_chat_message_learns_relationship_memory_from_clear_user_preferences() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let first = run_chat_message(
        "I prefer concise progress updates.",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            no_context: true,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("first chat");
    assert_eq!(first.relationship_hits, 0);
    assert_eq!(first.relationship_learned, 1);
    assert_eq!(first.emotion, EmotionState::Satisfied);

    let duplicate = run_chat_message(
        "I prefer concise progress updates.",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            no_context: true,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("duplicate chat");
    assert_eq!(duplicate.relationship_learned, 0);

    let snapshot =
        crate::relationship_snapshot(&paths, &workspace, Some("build"), Some("default"), 5)
            .await
            .expect("relationship snapshot");
    assert_eq!(snapshot.notes.len(), 1);
    assert_eq!(
        snapshot.notes[0].content,
        "User preference: concise progress updates"
    );

    let disabled = run_chat_message(
        "Call me Ikaros friend.",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            no_context: true,
            relationship_learning: false,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("disabled learning chat");
    assert_eq!(disabled.relationship_learned, 0);
}

#[tokio::test]
async fn run_chat_message_uses_explicit_mock_provider_for_offline_runtime_paths() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    let (session, registry) =
        crate::session_and_registry(&paths, &workspace, Some("build")).expect("session");
    session
        .execute_skill(
            &registry,
            "memory_append",
            serde_json::json!({
                "kind": "relationship",
                "scope": "default",
                "content": "prefers concise project status updates",
                "tags": ["relationship"],
            }),
        )
        .await
        .expect("relationship memory");

    let result = run_chat_message(
        "summarize the local runtime token=abc123",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions::default(),
    )
    .await
    .expect("chat");

    assert_eq!(result.provider, "mock");
    assert_eq!(result.emotion, EmotionState::Satisfied);
    assert!(!result.content.is_empty());
    assert!(!result.content.contains("abc123"));
    assert_eq!(result.relationship_hits, 1);
    assert_eq!(result.relationship_learned, 0);
    assert_eq!(result.history_hits, 0);
    assert!(result.audit_path.exists());
    assert!(result.model_usage_path.exists());
    assert!(result.chat_history_path.exists());
    assert!(!result.chat_session_id.is_empty());

    let second = run_chat_message(
        "continue from the previous turn",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            session_id: Some(result.chat_session_id.clone()),
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("second chat");
    assert_eq!(second.chat_session_id, result.chat_session_id);
    assert_eq!(second.history_hits, 1);
    assert_eq!(second.relationship_hits, 1);
    assert_eq!(second.relationship_learned, 0);
    assert_eq!(second.emotion, EmotionState::Satisfied);

    let isolated = run_chat_message(
        "start a different session",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            session_id: Some("isolated-session".into()),
            relationship_learning: false,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("isolated chat");
    assert_eq!(isolated.chat_session_id, "isolated-session");
    assert_eq!(isolated.history_hits, 0);

    let history = ChatHistoryStore::new(&paths.home)
        .read_all()
        .expect("chat history");
    assert_eq!(history.len(), 3);
    assert_eq!(history[0].session_id, result.chat_session_id);
    assert_eq!(history[0].agent, "build");
    assert_eq!(history[0].provider, "mock");
    assert_eq!(history[0].relationship_hits, 1);
    assert_eq!(history[0].memory_hits, result.memory_hits);
    assert!(!history[0].user_message.contains("abc123"));
    assert!(!history[0].assistant_message.contains("abc123"));
    assert!(history[0].user_message.contains("[REDACTED_SECRET]"));
    assert_eq!(history[1].session_id, second.chat_session_id);
    assert_eq!(history[1].relationship_hits, 1);
    assert_eq!(history[1].memory_hits, second.memory_hits);
    assert_eq!(history[2].session_id, "isolated-session");
    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let replay = session_store
        .replay_session(&SessionId::from(result.chat_session_id.clone()))
        .expect("session replay")
        .expect("persisted chat session");
    assert_eq!(replay.session.agent_id.as_deref(), Some("build"));
    assert_eq!(replay.entries.len(), 4);
    assert_eq!(replay.entries[0].kind, SessionEntryKind::UserMessage);
    assert_eq!(replay.entries[1].kind, SessionEntryKind::AssistantMessage);
    assert_eq!(replay.entries[2].kind, SessionEntryKind::UserMessage);
    assert_eq!(replay.entries[3].kind, SessionEntryKind::AssistantMessage);
    assert_eq!(
        replay.entries[1].parent_entry_id,
        Some(replay.entries[0].entry_id.clone())
    );
    assert_eq!(
        replay.entries[2].parent_entry_id,
        Some(replay.entries[1].entry_id.clone())
    );
    assert_eq!(
        replay.entries[3].parent_entry_id,
        Some(replay.entries[2].entry_id.clone())
    );
    assert_eq!(
        replay.entries[0]
            .turn_id
            .as_ref()
            .expect("first entry turn")
            .as_str(),
        history[0].turn_id
    );
    assert_eq!(
        replay.entries[2]
            .turn_id
            .as_ref()
            .expect("second entry turn")
            .as_str(),
        history[1].turn_id
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| event.turn_id.as_str() == history[0].turn_id
                && matches!(event.kind, AgentEventKind::TurnEnd))
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| event.turn_id.as_str() == history[1].turn_id
                && matches!(event.kind, AgentEventKind::TurnEnd))
    );
    let replay_json = serde_json::to_string(&replay).expect("replay json");
    assert!(!replay_json.contains("abc123"));
    assert!(replay_json.contains("[REDACTED_SECRET]"));
    let audit_events = ikaros_harness::AuditLog::new(&paths.audit_dir)
        .read_all()
        .expect("audit events");
    assert!(audit_events.iter().any(|event| {
        event.kind == crate::EMOTION_EVENT_KIND
            && event
                .data
                .get("emotion")
                .and_then(serde_json::Value::as_str)
                == Some("Satisfied")
    }));
    assert!(
        audit_events
            .iter()
            .any(|event| event.kind == "chat_history_recorded")
    );
    assert!(
        audit_events
            .iter()
            .any(|event| event.kind == "agent_loop_start")
    );
    assert!(
        audit_events
            .iter()
            .any(|event| event.kind == "agent_loop_end")
    );
}

#[tokio::test]
async fn run_chat_message_persists_single_call_chat_timeline() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let result = run_chat_message(
        "single call chat token=abc123",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            agent_loop: false,
            no_context: true,
            relationship_learning: false,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("single-call chat");

    assert_eq!(result.provider, "mock");
    assert_eq!(result.relationship_learned, 0);
    let history = ChatHistoryStore::new(&paths.home)
        .read_all()
        .expect("chat history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].session_id, result.chat_session_id);

    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let replay = session_store
        .replay_session(&SessionId::from(result.chat_session_id.clone()))
        .expect("session replay")
        .expect("persisted chat session");

    assert_eq!(replay.entries.len(), 2);
    assert_eq!(replay.entries[0].kind, SessionEntryKind::UserMessage);
    assert_eq!(replay.entries[1].kind, SessionEntryKind::AssistantMessage);
    assert_eq!(
        replay.entries[1].parent_entry_id,
        Some(replay.entries[0].entry_id.clone())
    );
    assert_eq!(
        replay.entries[0]
            .turn_id
            .as_ref()
            .expect("user entry turn")
            .as_str(),
        history[0].turn_id
    );
    assert_eq!(
        replay.entries[1]
            .turn_id
            .as_ref()
            .expect("assistant entry turn")
            .as_str(),
        history[0].turn_id
    );
    assert!(
        replay
            .agent_events
            .iter()
            .all(|event| event.turn_id.as_str() == history[0].turn_id)
    );
    assert!(matches!(
        replay.agent_events.first().map(|event| &event.kind),
        Some(AgentEventKind::SessionStart)
    ));
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::TurnStart))
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::UserMessage))
    );
    assert!(replay.agent_events.iter().any(|event| {
        matches!(
            &event.kind,
            AgentEventKind::ModelStream(ModelStreamEvent::Start { provider, .. })
                if provider == "mock"
        )
    }));
    assert!(replay.agent_events.iter().any(|event| {
        matches!(
            &event.kind,
            AgentEventKind::ModelStream(ModelStreamEvent::TextDelta(content))
                if content.contains("[REDACTED_SECRET]")
        )
    }));
    assert!(replay.agent_events.iter().any(|event| {
        matches!(
            &event.kind,
            AgentEventKind::ModelStream(ModelStreamEvent::Done)
        )
    }));
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::TurnEnd))
    );
    assert!(replay.agent_events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::MemoryLifecycle)
            && event.payload["phase"] == "sync_turn"
            && event.payload["status"] == "ok"
    }));
    assert!(replay.agent_events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::AuditAnchor)
            && event.payload["audit_path"].as_str().is_some()
            && event.payload["model_usage_path"].as_str().is_some()
    }));

    let replay_json = serde_json::to_string(&replay).expect("replay json");
    assert!(!replay_json.contains("abc123"));
    assert!(replay_json.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn run_chat_message_resolves_file_reference_and_persists_context_diff() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        workspace.join("notes.md"),
        "alpha reference line\nbeta reference line\ngamma omitted\n",
    )
    .expect("write reference");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let result = run_chat_message(
        "answer using @file:notes.md:1-2",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            agent_loop: false,
            relationship_learning: false,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("chat with reference");

    assert_eq!(result.reference_hits, 1);
    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let replay = session_store
        .replay_session(&SessionId::from(result.chat_session_id))
        .expect("session replay")
        .expect("persisted chat session");
    let context_event = replay
        .agent_events
        .iter()
        .find(|event| matches!(event.kind, AgentEventKind::ContextDiff))
        .expect("context diff event");
    assert_eq!(
        context_event.payload["references"][0]["raw"].as_str(),
        Some("@file:notes.md:1-2")
    );
    let sections = context_event.payload["sections"]
        .as_array()
        .expect("sections");
    assert!(sections.iter().any(|section| {
        section["kind"].as_str() == Some("references")
            && section["lines"][0]
                .as_str()
                .is_some_and(|line| line.contains("alpha reference line"))
    }));
    assert!(
        context_event.payload["diff"]["after_tokens"]
            .as_u64()
            .is_some_and(|tokens| tokens > 0)
    );
}

#[tokio::test]
async fn failed_single_call_provider_turn_is_replayable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    let execution = ikaros_harness::ExecutionSession::new(&workspace, &audit);
    let registry = ikaros_harness::SkillRegistry::new();
    let persona = PersonaLoader::parse(PersonaLoader::default_markdown()).expect("persona");
    let agent = ResolvedAgentProfile {
        name: "build".into(),
        profile: AgentProfile::build(),
    };
    let session_store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::new(temp.path()));
    let sink = PersistingAgentTurnSink::new(session_store.clone())
        .with_source(SessionSource::Cli)
        .with_agent_id("build")
        .with_workspace(&workspace);

    let error = run_chat_turn_with_events(
        "please fail token=abc123",
        &persona,
        &FailingProvider,
        &agent,
        &execution,
        &registry,
        ChatTurnEventOptions {
            options: &ChatRunOptions {
                agent_loop: false,
                no_context: true,
                session_id: Some("failed-provider-session".into()),
                ..ChatRunOptions::default()
            },
            event_sink: &sink,
            session_sink: Some(&sink),
            parent_entry_id: None,
            turn_id: None,
        },
    )
    .await
    .expect_err("provider should fail");
    assert!(error.to_string().contains("[REDACTED_SECRET]"));
    sink.commit().expect("commit failed timeline");

    let replay = session_store
        .replay_session(&SessionId::from("failed-provider-session"))
        .expect("replay")
        .expect("failed session exists");
    assert_eq!(replay.entries.len(), 1);
    assert_eq!(replay.entries[0].kind, SessionEntryKind::UserMessage);
    assert!(
        replay.entries[0]
            .visible_text
            .as_deref()
            .expect("visible text")
            .contains("[REDACTED_SECRET]")
    );
    assert!(replay.agent_events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::Error)
            && event
                .payload
                .get("phase")
                .and_then(serde_json::Value::as_str)
                == Some("provider_generate")
    }));
    assert!(matches!(
        replay.agent_events.last().map(|event| &event.kind),
        Some(AgentEventKind::TurnEnd)
    ));
    assert_eq!(
        replay.agent_events.last().and_then(|event| {
            event
                .payload
                .get("status")
                .and_then(serde_json::Value::as_str)
        }),
        Some("failed")
    );
    let replay_json = serde_json::to_string(&replay).expect("replay json");
    assert!(!replay_json.contains("abc123"));
    assert!(replay_json.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn failed_chat_history_write_keeps_replayable_turn() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);
    fs::create_dir_all(paths.home.join("chat/history.jsonl")).expect("history dir");

    let error = run_chat_message(
        "store failure token=abc123",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            agent_loop: false,
            no_context: true,
            relationship_learning: false,
            session_id: Some("history-failure-session".into()),
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect_err("chat history append should fail");
    assert!(error.to_string().contains("history.jsonl"));

    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let replay = session_store
        .replay_session(&SessionId::from("history-failure-session"))
        .expect("replay")
        .expect("failed session exists");
    assert_eq!(replay.entries.len(), 2);
    assert_eq!(replay.entries[0].kind, SessionEntryKind::UserMessage);
    assert_eq!(replay.entries[1].kind, SessionEntryKind::AssistantMessage);
    assert!(replay.agent_events.iter().any(|event| {
        matches!(event.kind, AgentEventKind::Error)
            && event
                .payload
                .get("phase")
                .and_then(serde_json::Value::as_str)
                == Some("chat_history_append")
    }));
    assert!(matches!(
        replay.agent_events.last().map(|event| &event.kind),
        Some(AgentEventKind::TurnEnd)
    ));
    assert_eq!(
        replay.agent_events.last().and_then(|event| {
            event
                .payload
                .get("status")
                .and_then(serde_json::Value::as_str)
        }),
        Some("failed")
    );
    let replay_json = serde_json::to_string(&replay).expect("replay json");
    assert!(!replay_json.contains("abc123"));
    assert!(replay_json.contains("[REDACTED_SECRET]"));
}

#[test]
fn chat_history_store_supports_sqlite_backend() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = ChatHistoryStore::new_with_backend(temp.path(), "sqlite").expect("sqlite store");
    let record = chat_history_record("session-1", "remember token=abc123");
    store.append(&record).expect("append");

    assert_eq!(store.backend_name(), "sqlite");
    assert!(store.path().ends_with("chat/history.sqlite"));
    assert!(store.path().exists());
    let records = store.read_all().expect("records");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].session_id, "session-1");
    assert!(!records[0].user_message.contains("abc123"));
    let context = store.recent_context_lines(1).expect("context");
    assert_eq!(context.len(), 1);
    assert!(context[0].contains("[REDACTED_SECRET]"));
}

#[test]
fn chat_history_store_filters_and_deletes_sessions_for_all_backends() {
    for backend in ["jsonl", "sqlite"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = ChatHistoryStore::new_with_backend(temp.path(), backend).expect("store");
        store
            .append(&chat_history_record("session-a", "first"))
            .expect("append a1");
        store
            .append(&chat_history_record("session-b", "second"))
            .expect("append b");
        store
            .append(&chat_history_record("session-a", "third"))
            .expect("append a2");

        let session_a = store.read_session("session-a").expect("session a");
        assert_eq!(session_a.len(), 2);
        assert!(
            session_a
                .iter()
                .all(|record| record.session_id == "session-a")
        );

        assert_eq!(
            store.delete_session("session-a").expect("delete session"),
            2
        );
        let remaining = store.read_all().expect("remaining");
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].session_id, "session-b");
        assert_eq!(store.clear().expect("clear"), 1);
        assert!(store.read_all().expect("empty").is_empty());
    }
}

#[test]
fn chat_history_store_searches_records_for_all_backends() {
    for backend in ["jsonl", "sqlite"] {
        let temp = tempfile::tempdir().expect("tempdir");
        let store = ChatHistoryStore::new_with_backend(temp.path(), backend).expect("store");
        store
            .append(&chat_history_record("session-a", "alpha first"))
            .expect("append alpha");
        store
            .append(&chat_history_record("session-b", "beta second"))
            .expect("append beta");
        store
            .append(&chat_history_record(
                "session-a",
                "alpha follow-up token=abc123",
            ))
            .expect("append redacted");

        let matches = store.search("alpha", 10, None).expect("search alpha");
        assert_eq!(matches.len(), 2);
        assert_eq!(matches[0].session_id, "session-a");
        assert!(matches[0].user_message.contains("follow-up"));
        assert!(!matches[0].user_message.contains("abc123"));
        assert!(matches[0].user_message.contains("[REDACTED_SECRET]"));

        let session_b = store
            .search("alpha", 10, Some("session-b"))
            .expect("search filtered");
        assert!(session_b.is_empty());

        let redacted = store
            .search("token=abc123", 10, Some("session-a"))
            .expect("search redacted");
        assert_eq!(redacted.len(), 1);
        assert!(redacted[0].user_message.contains("[REDACTED_SECRET]"));

        assert!(
            store
                .search("alpha", 0, None)
                .expect("zero limit")
                .is_empty()
        );
    }
}

fn chat_history_record(session_id: &str, user_message: &str) -> ChatHistoryRecord {
    super::history::build_chat_history_record_with_turn_id(
        uuid::Uuid::new_v4().to_string(),
        super::history::ChatHistoryAppend {
            session_id,
            agent: "build",
            provider: "mock",
            model: "mock-ikaros",
            streamed: false,
            user_message,
            assistant_message: "stored safely",
            relationship_hits: 0,
            memory_hits: 0,
            rag_hits: 0,
        },
    )
    .expect("record")
}

fn write_offline_mock_config(paths: &IkarosPaths) {
    fs::create_dir_all(&paths.home).expect("home");
    fs::write(
        &paths.config,
        r#"model:
  default:
    provider: mock
    runtime: harness-agent-loop
    transport: mock
    model: mock-ikaros

rag:
  embedding_provider: hash
  embedding_model: text-embedding-3-small
"#,
    )
    .expect("mock config");
}
