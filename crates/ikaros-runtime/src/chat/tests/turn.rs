// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn run_chat_message_creates_relationship_candidate_from_clear_user_preferences() {
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
    assert_eq!(first.relationship_candidates_created, 1);
    assert_eq!(first.emotion, EmotionState::Satisfied);
    let candidate_store = JsonlMemoryCandidateStore::new(&paths.memory_dir);
    let pending = candidate_store
        .list(ikaros_memory::MemoryCandidateQuery {
            status: Some(MemoryCandidateStatus::Pending),
            ..ikaros_memory::MemoryCandidateQuery::default()
        })
        .expect("pending candidates");
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].scope, "default");
    assert_eq!(
        pending[0].content,
        "User preference: concise progress updates"
    );

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
    assert_eq!(duplicate.relationship_candidates_created, 0);

    let snapshot =
        crate::relationship_snapshot(&paths, &workspace, Some("build"), Some("default"), 5)
            .await
            .expect("relationship snapshot");
    assert!(snapshot.notes.is_empty());

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
    assert_eq!(disabled.relationship_candidates_created, 0);
}

#[tokio::test]
async fn agent_loop_chat_uses_configured_model_request_options() {
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
    let provider = RecordingOptionsProvider::default();
    let request_options = ModelRequestOptions {
        max_tokens: Some(8_192),
        temperature: Some(0.2),
        ..ModelRequestOptions::default()
    };

    run_chat_turn_with_events(
        "hello",
        &persona,
        &provider,
        &agent,
        &execution,
        &registry,
        ChatTurnEventOptions {
            options: &ChatRunOptions {
                agent_loop: true,
                stream: false,
                no_context: true,
                relationship_learning: false,
                session_id: Some("request-options-session".into()),
                ..ChatRunOptions::default()
            },
            request_options: Some(&request_options),
            event_sink: noop_agent_event_sink(),
            session_sink: None,
            parent_entry_id: None,
            turn_id: None,
        },
    )
    .await
    .expect("agent-loop chat turn");

    assert_eq!(
        *provider.max_tokens.lock().expect("max tokens"),
        Some(8_192)
    );
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
    assert_eq!(result.relationship_candidates_created, 0);
    assert_eq!(result.history_hits, 0);
    assert!(result.audit_path.exists());
    assert!(result.model_usage_path.exists());
    assert!(result.session_state_db.exists());
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
    assert_eq!(second.relationship_candidates_created, 0);
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

    assert!(
        !paths.home.join("chat/history.jsonl").exists(),
        "ordinary chat turns should not write a legacy history mirror"
    );
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
    let projected = super::super::history::chat_history_records_from_session_replay(&replay);
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0].session_id, result.chat_session_id);
    assert_eq!(projected[0].agent, "build");
    assert_eq!(projected[0].provider, "mock");
    assert_eq!(projected[0].relationship_hits, 1);
    assert_eq!(projected[0].memory_hits, result.memory_hits);
    assert!(!projected[0].user_message.contains("abc123"));
    assert!(!projected[0].assistant_message.contains("abc123"));
    assert!(projected[0].user_message.contains("[REDACTED_SECRET]"));
    assert_eq!(projected[1].session_id, second.chat_session_id);
    assert_eq!(projected[1].relationship_hits, 1);
    assert_eq!(projected[1].memory_hits, second.memory_hits);
    let isolated_replay = session_store
        .replay_session(&SessionId::from("isolated-session"))
        .expect("isolated replay")
        .expect("isolated session");
    let isolated_projected =
        super::super::history::chat_history_records_from_session_replay(&isolated_replay);
    assert_eq!(isolated_projected.len(), 1);
    assert_eq!(
        replay.entries[0]
            .turn_id
            .as_ref()
            .expect("first entry turn")
            .as_str(),
        projected[0].turn_id
    );
    assert_eq!(
        replay.entries[2]
            .turn_id
            .as_ref()
            .expect("second entry turn")
            .as_str(),
        projected[1].turn_id
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| event.turn_id.as_str() == projected[0].turn_id
                && matches!(event.kind, AgentEventKind::TurnEnd))
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| event.turn_id.as_str() == projected[1].turn_id
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
            .any(|event| event.kind == "agent_loop_start")
    );
    assert!(
        audit_events
            .iter()
            .any(|event| event.kind == "agent_loop_end")
    );
}

#[tokio::test]
async fn run_chat_message_uses_session_store_without_legacy_history_mirror() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let first = run_chat_message(
        "first session-store-only history turn",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            relationship_learning: false,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("first chat");
    let second = run_chat_message(
        "continue from session replay only",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            session_id: Some(first.chat_session_id.clone()),
            relationship_learning: false,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("second chat");

    assert_eq!(second.chat_session_id, first.chat_session_id);
    assert_eq!(second.history_hits, 1);
    assert!(
        !paths.home.join("chat/history.jsonl").exists(),
        "runtime chat should not keep writing the legacy chat history mirror"
    );

    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let replay = session_store
        .replay_session(&SessionId::from(first.chat_session_id.clone()))
        .expect("session replay")
        .expect("persisted chat session");
    let projected = super::super::history::chat_history_records_from_session_replay(&replay);
    assert_eq!(projected.len(), 2);
    assert_eq!(projected[0].session_id, first.chat_session_id);
    assert_eq!(projected[1].session_id, first.chat_session_id);
}

#[tokio::test]
async fn run_chat_message_reads_history_context_from_session_replay() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let first = run_chat_message(
        "first replay-backed context turn token=abc123",
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
    .expect("first chat");

    let second = run_chat_message(
        "continue using replay-backed history",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            session_id: Some(first.chat_session_id.clone()),
            relationship_learning: false,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("second chat");

    assert_eq!(second.chat_session_id, first.chat_session_id);
    assert_eq!(second.history_hits, 1);

    let session_store = SqliteSessionStore::new(paths.home.join("agents").join("build"));
    let replay = session_store
        .replay_session(&SessionId::from(first.chat_session_id.clone()))
        .expect("replay")
        .expect("session replay");
    let mut tombstone = ikaros_session::SessionEntry::new(
        replay.session.session_id.clone(),
        SessionEntryKind::Custom,
    );
    tombstone.parent_entry_id = replay.entries.last().map(|entry| entry.entry_id.clone());
    tombstone.payload = serde_json::json!({
        "operation": super::super::history::CHAT_HISTORY_DELETE_SESSION_OPERATION,
    });
    session_store
        .append_entry(&tombstone)
        .expect("append tombstone");

    let hidden = run_chat_message(
        "continue after hidden history",
        &paths,
        &workspace,
        Some("build"),
        ChatRunOptions {
            session_id: Some(first.chat_session_id.clone()),
            relationship_learning: false,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("hidden history chat");
    assert_eq!(hidden.history_hits, 0);
}

#[test]
fn session_replay_history_projection_skips_assistant_entries_without_model_identity() {
    let session_id = SessionId::from("projection-missing-model");
    let turn_id = TurnId::from("turn-with-missing-model");
    let mut user = SessionEntry::new(session_id.clone(), SessionEntryKind::UserMessage);
    user.turn_id = Some(turn_id.clone());
    user.visible_text = Some("hello".into());
    let mut assistant = SessionEntry::new(session_id.clone(), SessionEntryKind::AssistantMessage);
    assistant.parent_entry_id = Some(user.entry_id.clone());
    assistant.turn_id = Some(turn_id);
    assistant.visible_text = Some("hi".into());
    assistant.payload = serde_json::json!({
        "agent": "build",
        "provider": "mock"
    });
    let replay = SessionReplay {
        session: SessionRecord::new(session_id, SessionSource::Test),
        entries: vec![user, assistant],
        agent_events: Vec::new(),
        approvals: Vec::new(),
    };

    let projected = super::super::history::chat_history_records_from_session_replay(&replay);

    assert!(
        projected.is_empty(),
        "assistant entries missing provider/model identity must not become empty-provider history records: {projected:#?}"
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
    assert_eq!(result.relationship_candidates_created, 0);
    assert!(!paths.home.join("chat/history.jsonl").exists());

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
        replay.entries[0].turn_id.as_ref().expect("user entry turn"),
        replay.entries[1]
            .turn_id
            .as_ref()
            .expect("assistant entry turn")
    );
    let turn_id = replay.entries[0].turn_id.as_ref().expect("turn id");
    let inputs = session_store
        .session_inputs(&SessionId::from(result.chat_session_id.clone()))
        .expect("session inputs");
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].status, SessionInputStatus::Promoted);
    assert_eq!(inputs[0].promoted_turn_id.as_ref(), Some(turn_id));
    assert_eq!(
        inputs[0].payload["content"].as_str(),
        Some("single call chat token=[REDACTED_SECRET]")
    );
    assert!(
        replay
            .agent_events
            .iter()
            .all(|event| &event.turn_id == turn_id)
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
    assert_eq!(
        context_event.payload["budget"]["source"].as_str(),
        Some("mock")
    );
    assert_eq!(
        context_event.payload["budget"]["estimator"].as_str(),
        Some("mock-tokenizer-v1")
    );
    assert_eq!(
        context_event.payload["budget"]["context_window"].as_u64(),
        Some(8_192)
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
    let prompt_sections = context_event.payload["prompt_sections"]
        .as_array()
        .expect("prompt sections");
    assert!(
        context_event.payload["prompt_stable_prefix_hash"]
            .as_str()
            .is_some_and(|hash| hash.starts_with("fnv1a64:"))
    );
    assert!(
        context_event.payload["prompt_stable_prefix_message_count"]
            .as_u64()
            .is_some_and(|count| count > 0)
    );
    assert!(
        context_event.payload["prompt_stable_prefix_estimated_tokens"]
            .as_u64()
            .is_some_and(|tokens| tokens > 0)
    );
    assert!(prompt_sections.iter().any(|section| {
        section["kind"].as_str() == Some("references")
            && section["source"].as_str() == Some("context")
            && section["estimated_tokens"]
                .as_u64()
                .is_some_and(|tokens| tokens > 0)
    }));
    assert!(
        prompt_sections
            .iter()
            .all(|section| section.get("content").is_none()),
        "persisted prompt sections should expose metadata only, not full prompt content: {prompt_sections:#?}"
    );
    assert!(prompt_sections.iter().any(|section| {
        section["kind"].as_str() == Some("tool_guidance")
            && section["source"].as_str() == Some("tooling")
    }));
    assert!(
        context_event.payload["diff"]["after_tokens"]
            .as_u64()
            .is_some_and(|tokens| tokens > 0)
    );
}

#[tokio::test]
async fn run_chat_message_handles_binary_file_reference_as_context_notice() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(workspace.join("image.bin"), [0xff, 0x00, 0x89, 0x50])
        .expect("write binary reference");
    let paths = IkarosPaths::from_home(home);
    write_offline_mock_config(&paths);

    let result = run_chat_message(
        "inspect @file:image.bin without failing the turn",
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
    .expect("chat with binary reference");

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
        Some("@file:image.bin")
    );
    let sections = context_event.payload["sections"]
        .as_array()
        .expect("sections");
    assert!(sections.iter().any(|section| {
        section["kind"].as_str() == Some("references")
            && section["lines"][0]
                .as_str()
                .is_some_and(|line| line.contains("skipped: non-text or non-utf8 file bytes=4"))
    }));
}

#[tokio::test]
async fn tiny_context_window_persists_compaction_entry_and_event() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    let session_id = SessionId::from("tiny-context-session");
    let session_store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::new(temp.path()));
    session_store
        .upsert_session(&SessionRecord::new(session_id.clone(), SessionSource::Test))
        .expect("seed session");
    let mut parent_entry_id = None;
    for index in 0..18 {
        let turn_id = TurnId::from(format!("history-turn-{index}"));
        let mut user = SessionEntry::new(session_id.clone(), SessionEntryKind::UserMessage);
        user.parent_entry_id = parent_entry_id.clone();
        user.turn_id = Some(turn_id.clone());
        user.visible_text = Some(format!("older user turn {index}"));
        user.payload = serde_json::json!({
            "role": "user",
            "content": user.visible_text.clone(),
        });
        session_store.append_entry(&user).expect("append user");

        let mut assistant =
            SessionEntry::new(session_id.clone(), SessionEntryKind::AssistantMessage);
        assistant.parent_entry_id = Some(user.entry_id.clone());
        assistant.turn_id = Some(turn_id);
        assistant.visible_text = Some(format!(
            "older turn {index} with enough repeated words to exceed the tiny context window"
        ));
        assistant.payload = serde_json::json!({
            "role": "assistant",
            "agent": "plan",
            "provider": "tiny-window",
            "model": "tiny-window-model",
            "content": assistant.visible_text.clone(),
        });
        parent_entry_id = Some(assistant.entry_id.clone());
        session_store
            .append_entry(&assistant)
            .expect("append assistant");
    }
    let execution = ikaros_harness::ExecutionSession::new(&workspace, &audit);
    let registry = ikaros_harness::SkillRegistry::new();
    let persona = PersonaLoader::parse(PersonaLoader::default_markdown()).expect("persona");
    let mut profile = AgentProfile::plan();
    profile.memory_context = false;
    profile.rag_context = false;
    let agent = ResolvedAgentProfile {
        name: "plan".into(),
        profile,
    };
    let sink = PersistingAgentTurnSink::new(session_store.clone())
        .with_source(SessionSource::Cli)
        .with_agent_id("plan")
        .with_workspace(&workspace);

    run_chat_turn_with_events(
        "summarize the previous discussion",
        &persona,
        &TinyWindowProvider,
        &agent,
        &execution,
        &registry,
        ChatTurnEventOptions {
            options: &ChatRunOptions {
                agent_loop: false,
                relationship_learning: false,
                session_id: Some("tiny-context-session".into()),
                session_state_db: Some(temp.path().join("state.db")),
                history_context_limit: 18,
                history_summary_limit: 0,
                ..ChatRunOptions::default()
            },
            request_options: None,
            event_sink: &sink,
            session_sink: Some(&sink),
            parent_entry_id: None,
            turn_id: None,
        },
    )
    .await
    .expect("chat turn");
    sink.commit().expect("commit");

    let replay = session_store
        .replay_session(&SessionId::from("tiny-context-session"))
        .expect("replay")
        .expect("session");
    assert!(
        replay
            .entries
            .iter()
            .any(|entry| entry.kind == SessionEntryKind::Compaction)
    );
    let compaction_entry = replay
        .entries
        .iter()
        .find(|entry| entry.kind == SessionEntryKind::Compaction)
        .expect("compaction entry");
    assert!(
        compaction_entry.payload["continuation_prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.contains("do not invent omitted details"))
    );
    assert!(
        replay
            .agent_events
            .iter()
            .any(|event| matches!(event.kind, AgentEventKind::ContextCompacted))
    );
    let compaction_event = replay
        .agent_events
        .iter()
        .find(|event| matches!(event.kind, AgentEventKind::ContextCompacted))
        .expect("compaction event");
    assert!(
        compaction_event.payload["continuation_prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.contains("Compacted sections"))
    );
}

#[tokio::test]
async fn llm_summary_context_engine_persists_provider_backed_compaction() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    let session_id = SessionId::from("llm-summary-context-session");
    let session_store: Arc<dyn SessionStore> = Arc::new(SqliteSessionStore::new(temp.path()));
    session_store
        .upsert_session(&SessionRecord::new(session_id.clone(), SessionSource::Test))
        .expect("seed session");
    let mut parent_entry_id = None;
    for index in 0..18 {
        let turn_id = TurnId::from(format!("summary-history-turn-{index}"));
        let mut user = SessionEntry::new(session_id.clone(), SessionEntryKind::UserMessage);
        user.parent_entry_id = parent_entry_id.clone();
        user.turn_id = Some(turn_id.clone());
        user.visible_text = Some(format!("older user turn {index}"));
        user.payload = serde_json::json!({
            "role": "user",
            "content": user.visible_text.clone(),
        });
        session_store.append_entry(&user).expect("append user");

        let mut assistant =
            SessionEntry::new(session_id.clone(), SessionEntryKind::AssistantMessage);
        assistant.parent_entry_id = Some(user.entry_id.clone());
        assistant.turn_id = Some(turn_id);
        assistant.visible_text = Some(format!(
            "older assistant turn {index} with enough repeated words to exceed the tiny context window token=sk-secret-value"
        ));
        assistant.payload = serde_json::json!({
            "role": "assistant",
            "agent": "plan",
            "provider": "provider-summary",
            "model": "provider-summary-model",
            "content": assistant.visible_text.clone(),
        });
        parent_entry_id = Some(assistant.entry_id.clone());
        session_store
            .append_entry(&assistant)
            .expect("append assistant");
    }
    let execution = ikaros_harness::ExecutionSession::new(&workspace, &audit);
    let registry = ikaros_harness::SkillRegistry::new();
    let persona = PersonaLoader::parse(PersonaLoader::default_markdown()).expect("persona");
    let mut profile = AgentProfile::plan();
    profile.memory_context = false;
    profile.rag_context = false;
    let agent = ResolvedAgentProfile {
        name: "plan".into(),
        profile,
    };
    let sink = PersistingAgentTurnSink::new(session_store.clone())
        .with_source(SessionSource::Cli)
        .with_agent_id("plan")
        .with_workspace(&workspace);
    let provider = ProviderBackedSummaryProvider::default();

    run_chat_turn_with_events(
        "summarize with provider-backed compression",
        &persona,
        &provider,
        &agent,
        &execution,
        &registry,
        ChatTurnEventOptions {
            options: &ChatRunOptions {
                agent_loop: false,
                relationship_learning: false,
                session_id: Some("llm-summary-context-session".into()),
                session_state_db: Some(temp.path().join("state.db")),
                history_context_limit: 18,
                history_summary_limit: 0,
                context_engine: Some("llm-summary".into()),
                ..ChatRunOptions::default()
            },
            request_options: None,
            event_sink: &sink,
            session_sink: Some(&sink),
            parent_entry_id: None,
            turn_id: None,
        },
    )
    .await
    .expect("chat turn");
    sink.commit().expect("commit");

    assert_eq!(provider.summary_calls.load(Ordering::SeqCst), 1);
    let replay = session_store
        .replay_session(&SessionId::from("llm-summary-context-session"))
        .expect("replay")
        .expect("session");
    let compaction_event = replay
        .agent_events
        .iter()
        .find(|event| matches!(event.kind, AgentEventKind::ContextCompacted))
        .expect("compaction event");
    assert!(
        compaction_event.payload["continuation_prompt"]
            .as_str()
            .is_some_and(|prompt| prompt.contains("LLM summary compressor"))
    );
    assert!(
        compaction_event.payload["summary"]
            .as_str()
            .is_some_and(|summary| summary.contains("provider summary"))
    );
    assert!(
        !serde_json::to_string(compaction_event)
            .expect("compaction event json")
            .contains("sk-secret-value")
    );
}

#[tokio::test]
async fn oversized_explicit_reference_is_truncated_before_context_assembly() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        workspace.join("huge.md"),
        (0..48)
            .map(|index| {
                format!(
                    "protected reference line {index} must not be silently removed from context"
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .expect("reference");
    let execution = ikaros_harness::ExecutionSession::new(&workspace, &audit);
    let registry = ikaros_harness::SkillRegistry::new();
    let mut profile = AgentProfile::plan();
    profile.memory_context = false;
    profile.rag_context = false;
    let agent = ResolvedAgentProfile {
        name: "plan".into(),
        profile,
    };
    let model_context =
        ModelContextProfile::new(900, 100, ModelTokenizerKind::Mock, "reference-cap-test");
    let bundle = build_chat_context_bundle_with_model_context(
        &LocalChatContextEngine,
        "summarize @file:huge.md",
        &agent,
        &execution,
        &registry,
        &ChatRunOptions {
            context_token_budget: 10_000,
            relationship_learning: false,
            ..ChatRunOptions::default()
        },
        ContextModelBudget {
            model_context: &model_context,
            reserved_system_tokens: 16,
        },
    )
    .await
    .expect("oversized protected reference should be truncated");

    assert_eq!(bundle.references.len(), 1);
    assert_eq!(bundle.budget.max_tokens, 784);
    let reference_text = bundle.context.references.join("\n");
    assert!(reference_text.contains("truncated: explicit references capped at 50% context budget"));
    assert!(!reference_text.contains("protected reference line 47"));
    let estimator = ikaros_context::ContextTokenizerKind::Mock.estimator();
    let reference_tokens = bundle
        .context
        .references
        .iter()
        .map(|reference| estimator.estimate_tokens(reference))
        .sum::<usize>();
    assert!(reference_tokens <= bundle.budget.max_tokens / 2);
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
            request_options: None,
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
async fn single_call_chat_cancellation_skips_provider_request() {
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
    let provider = CountingProvider::default();
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = run_chat_turn(
        "cancel before provider",
        &persona,
        &provider,
        &agent,
        &execution,
        &registry,
        &ChatRunOptions {
            agent_loop: false,
            no_context: true,
            session_id: Some("cancelled-single-call-session".into()),
            cancellation,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect_err("cancelled chat should fail before provider request");

    assert!(error.to_string().contains("cancelled"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}
