// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn unknown_chat_context_engine_is_rejected() {
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

    let error = run_chat_turn_with_events(
        "hello",
        &persona,
        &provider,
        &agent,
        &execution,
        &registry,
        ChatTurnEventOptions {
            options: &ChatRunOptions {
                context_engine: Some("missing-engine".into()),
                relationship_learning: false,
                ..ChatRunOptions::default()
            },
            request_options: None,
            event_sink: noop_agent_event_sink(),
            session_sink: None,
            parent_entry_id: None,
            turn_id: None,
        },
    )
    .await
    .expect_err("unknown context engine should fail fast");

    assert!(error.to_string().contains("unknown context engine"));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[test]
fn chat_prompt_builder_skips_empty_optional_context_sections() {
    let context = ContextBuilder::new()
        .persona_context("Persona without optional context")
        .build();

    let report = build_chat_system_prompt(&context, &HeuristicTokenEstimator);

    assert!(report.prompt.contains("Persona without optional context"));
    assert!(report.prompt.contains("Policy:"));
    assert!(report.prompt.contains("Tool guidance:"));
    for empty_title in [
        "Local relationship context",
        "Local reference context",
        "Local chat history context",
        "Accepted memory projection",
        "Session working memory",
        "Retrieved memory context",
        "Local RAG context",
    ] {
        assert!(
            !report.prompt.contains(empty_title),
            "empty optional prompt section leaked into prompt: {empty_title}\n{}",
            report.prompt
        );
    }
    assert!(!report.prompt.contains(":\nnone"));
    for empty_kind in [
        PromptSectionKind::Relationship,
        PromptSectionKind::References,
        PromptSectionKind::History,
        PromptSectionKind::MemoryProjection,
        PromptSectionKind::WorkingMemory,
        PromptSectionKind::RetrievedMemory,
        PromptSectionKind::Rag,
    ] {
        assert!(
            report
                .sections
                .iter()
                .all(|section| section.kind != empty_kind),
            "empty optional section should not appear in prompt metadata: {empty_kind:?}"
        );
    }
    assert!(
        report
            .sections
            .iter()
            .any(|section| section.kind == PromptSectionKind::Persona)
    );
    assert!(
        report
            .sections
            .iter()
            .any(|section| section.kind == PromptSectionKind::Policy)
    );
    assert!(
        report
            .sections
            .iter()
            .any(|section| section.kind == PromptSectionKind::ToolGuidance)
    );
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
fn chat_context_budget_preserves_priority_and_omits_low_sections() {
    let context = ChatContext {
        relationship: vec!["rel".into()],
        references: vec!["ref".into()],
        history: vec!["hist".into()],
        retrieved_memory: vec!["memory-context-is-long-enough-to-truncate".into()],
        rag: vec!["rag should be omitted".into()],
        ..ChatContext::default()
    };

    let estimator = HeuristicTokenEstimator;
    let budgeted = apply_context_token_budget(
        super::super::context::redact_chat_context(context),
        12,
        &estimator,
    );

    assert_eq!(budgeted.relationship, vec!["rel"]);
    assert_eq!(budgeted.references, vec!["ref"]);
    assert_eq!(budgeted.history, vec!["hist"]);
    assert!(budgeted.retrieved_memory.is_empty());
    assert!(budgeted.rag.is_empty());
    assert!(chat_context_token_count(&budgeted, &estimator) <= 12);
}

#[test]
fn chat_context_budget_zero_keeps_context_unbounded() {
    let context = ChatContext {
        relationship: vec!["rel".into()],
        references: vec!["ref".into()],
        history: vec!["hist".into()],
        memory_projection: vec!["projection".into()],
        working_memory: vec!["working".into()],
        retrieved_memory: vec!["memory".into()],
        rag: vec!["rag".into()],
    };

    assert_eq!(
        apply_context_token_budget(
            super::super::context::redact_chat_context(context.clone()),
            0,
            &HeuristicTokenEstimator,
        ),
        context
    );
}

#[test]
fn model_tokenizer_kind_selects_context_estimator() {
    use super::super::context_engine::context_estimator_for_model;

    let openai = ModelContextProfile::new(
        128_000,
        4_096,
        ModelTokenizerKind::OpenAiCompatible,
        "openai-compatible",
    );
    let anthropic =
        ModelContextProfile::new(200_000, 8_192, ModelTokenizerKind::Anthropic, "anthropic");
    let ollama = ModelContextProfile::new(32_768, 2_048, ModelTokenizerKind::Ollama, "ollama");
    let mock = ModelContextProfile::new(8_192, 1_024, ModelTokenizerKind::Mock, "mock");

    assert_eq!(
        context_estimator_for_model(Some(&openai)).name(),
        "openai-compatible-chatml-v1"
    );
    assert_eq!(
        context_estimator_for_model(Some(&anthropic)).name(),
        "anthropic-fallback-heuristic-v1"
    );
    assert_eq!(
        context_estimator_for_model(Some(&ollama)).name(),
        "ollama-fallback-heuristic-v1"
    );
    assert_eq!(
        context_estimator_for_model(Some(&mock)).name(),
        "mock-tokenizer-v1"
    );
    assert_eq!(context_estimator_for_model(None).name(), "heuristic-v1");
}

#[tokio::test]
async fn provider_context_window_caps_chat_context_budget() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        workspace.join("notes.md"),
        "alpha reference line\nbeta reference line\ngamma reference line\n",
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
    let model_context = ModelContextProfile::new(96, 32, ModelTokenizerKind::Mock, "tiny-test");
    let bundle = build_chat_context_bundle_with_model_context(
        &LocalChatContextEngine,
        "use @file:notes.md",
        &agent,
        &execution,
        &registry,
        &ChatRunOptions {
            context_token_budget: 2_000,
            ..ChatRunOptions::default()
        },
        ContextModelBudget {
            model_context: &model_context,
            reserved_system_tokens: 16,
        },
    )
    .await
    .expect("context bundle");

    assert_eq!(bundle.budget.max_tokens, 48);
    assert_eq!(bundle.budget.estimator, "mock-tokenizer-v1");
    assert_eq!(bundle.budget.requested_tokens, Some(2_000));
    assert_eq!(bundle.budget.context_window, Some(96));
    assert_eq!(bundle.budget.reserved_output_tokens, Some(32));
    assert_eq!(bundle.budget.reserved_system_tokens, Some(16));
    assert_eq!(bundle.budget.source.as_deref(), Some("tiny-test"));
    assert!(bundle.budget.used_tokens <= bundle.budget.max_tokens);
}

#[tokio::test]
async fn url_context_reference_uses_execution_env_network_egress() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    let network = GovernedNetworkEgress::new(
        NetworkEgressPolicy::allow_hosts(["docs.example".into()]),
        Arc::new(FixtureNetworkEgress),
    );
    let execution =
        ikaros_harness::ExecutionSession::new(&workspace, &audit).with_execution_env(Arc::new(
            NetworkedExecutionEnv::new(Arc::new(LocalExecutionEnv), Arc::new(network)),
        ));
    let registry = ikaros_harness::SkillRegistry::new();
    let mut profile = AgentProfile::plan();
    profile.memory_context = false;
    profile.rag_context = false;
    let agent = ResolvedAgentProfile {
        name: "plan".into(),
        profile,
    };
    let model_context = ModelContextProfile::new(512, 32, ModelTokenizerKind::Mock, "url-test");
    let bundle = build_chat_context_bundle_with_model_context(
        &LocalChatContextEngine,
        "inspect @url:https://docs.example/guide?token=abc123",
        &agent,
        &execution,
        &registry,
        &ChatRunOptions::default(),
        ContextModelBudget {
            model_context: &model_context,
            reserved_system_tokens: 16,
        },
    )
    .await
    .expect("url context bundle");

    let references = bundle.context.references.join("\n");
    assert!(references.contains("[reference/url]"));
    assert!(references.contains("status=200"));
    assert!(references.contains("remote docs"));
    assert!(!references.contains("abc123"));
    assert!(references.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn url_context_reference_skips_non_text_content_type() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    let network = GovernedNetworkEgress::new(
        NetworkEgressPolicy::allow_hosts(["docs.example".into()]),
        Arc::new(FixedNetworkEgress {
            response: NetworkEgressResponse {
                status: 200,
                headers: BTreeMap::from([(
                    "content-type".into(),
                    "text/html; charset=utf-8".into(),
                )]),
                body: "<html><body>do not inject this html</body></html>".into(),
                body_bytes: None,
            },
        }),
    );
    let execution =
        ikaros_harness::ExecutionSession::new(&workspace, &audit).with_execution_env(Arc::new(
            NetworkedExecutionEnv::new(Arc::new(LocalExecutionEnv), Arc::new(network)),
        ));
    let registry = ikaros_harness::SkillRegistry::new();
    let mut profile = AgentProfile::plan();
    profile.memory_context = false;
    profile.rag_context = false;
    let agent = ResolvedAgentProfile {
        name: "plan".into(),
        profile,
    };
    let model_context = ModelContextProfile::new(512, 32, ModelTokenizerKind::Mock, "url-test");
    let bundle = build_chat_context_bundle_with_model_context(
        &LocalChatContextEngine,
        "inspect @url:https://docs.example/page",
        &agent,
        &execution,
        &registry,
        &ChatRunOptions::default(),
        ContextModelBudget {
            model_context: &model_context,
            reserved_system_tokens: 16,
        },
    )
    .await
    .expect("url context bundle");

    let references = bundle.context.references.join("\n");
    assert!(references.contains("skipped: unsupported content-type text/html"));
    assert!(!references.contains("do not inject this html"));
}

#[tokio::test]
async fn url_context_reference_skips_oversized_body() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    let network = GovernedNetworkEgress::new(
        NetworkEgressPolicy::allow_hosts(["docs.example".into()]),
        Arc::new(FixedNetworkEgress {
            response: NetworkEgressResponse {
                status: 200,
                headers: BTreeMap::from([("content-type".into(), "text/plain".into())]),
                body: format!(
                    "start-secret sk-{}\n{}",
                    "a".repeat(32),
                    "x".repeat(80 * 1024)
                ),
                body_bytes: None,
            },
        }),
    );
    let execution =
        ikaros_harness::ExecutionSession::new(&workspace, &audit).with_execution_env(Arc::new(
            NetworkedExecutionEnv::new(Arc::new(LocalExecutionEnv), Arc::new(network)),
        ));
    let registry = ikaros_harness::SkillRegistry::new();
    let mut profile = AgentProfile::plan();
    profile.memory_context = false;
    profile.rag_context = false;
    let agent = ResolvedAgentProfile {
        name: "plan".into(),
        profile,
    };
    let model_context = ModelContextProfile::new(512, 32, ModelTokenizerKind::Mock, "url-test");
    let bundle = build_chat_context_bundle_with_model_context(
        &LocalChatContextEngine,
        "inspect @url:https://docs.example/large.txt",
        &agent,
        &execution,
        &registry,
        &ChatRunOptions::default(),
        ContextModelBudget {
            model_context: &model_context,
            reserved_system_tokens: 16,
        },
    )
    .await
    .expect("url context bundle");

    let references = bundle.context.references.join("\n");
    assert!(references.contains("skipped: response body too large bytes="));
    assert!(references.contains("max_bytes="));
    assert!(!references.contains("start-secret"));
    assert!(!references.contains("sk-"));
}

#[tokio::test]
async fn url_context_reference_denied_by_execution_env_network_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    let execution = ikaros_harness::ExecutionSession::new(&workspace, &audit).with_execution_env(
        Arc::new(NetworkedExecutionEnv::new(
            Arc::new(LocalExecutionEnv),
            Arc::new(GovernedNetworkEgress::deny_by_default()),
        )),
    );
    let registry = ikaros_harness::SkillRegistry::new();
    let mut profile = AgentProfile::plan();
    profile.memory_context = false;
    profile.rag_context = false;
    let agent = ResolvedAgentProfile {
        name: "plan".into(),
        profile,
    };
    let model_context = ModelContextProfile::new(512, 32, ModelTokenizerKind::Mock, "url-test");
    let error = build_chat_context_bundle_with_model_context(
        &LocalChatContextEngine,
        "inspect @url:https://blocked.example/guide",
        &agent,
        &execution,
        &registry,
        &ChatRunOptions::default(),
        ContextModelBudget {
            model_context: &model_context,
            reserved_system_tokens: 16,
        },
    )
    .await
    .expect_err("url context should respect deny policy");

    assert!(error.to_string().contains("network egress denied"));
    assert!(!error.to_string().contains("abc123"));
}

#[test]
fn chat_history_projection_summarizes_older_turns() {
    let records = vec![
        chat_history_record("session-a", "first long-running topic"),
        chat_history_record("session-a", "second continuity point"),
        chat_history_record("session-a", "third recent turn"),
    ];

    let lines = super::super::history::chat_history_context_lines_with_summary(&records, 1, 4);

    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("older chat summary turns=2"));
    assert!(lines[0].contains("first long-running topic"));
    assert!(lines[0].contains("second continuity point"));
    assert!(lines[1].contains("third recent turn"));
}

#[test]
fn chat_history_projection_can_be_scoped_to_session() {
    let records = vec![
        chat_history_record("session-a", "alpha first"),
        chat_history_record("session-b", "beta should stay out"),
        chat_history_record("session-a", "alpha recent"),
    ]
    .into_iter()
    .filter(|record| record.session_id == "session-a")
    .collect::<Vec<_>>();

    let lines = super::super::history::chat_history_context_lines_with_summary(&records, 1, 4);

    assert_eq!(lines.len(), 2);
    assert!(lines[0].contains("older chat summary turns=1"));
    assert!(lines[0].contains("alpha first"));
    assert!(lines[1].contains("alpha recent"));
    assert!(lines.iter().all(|line| !line.contains("beta")));
}

#[test]
fn chat_history_session_summaries_group_recent_session_replay_records() {
    let replay = SessionReplay {
        session: SessionRecord::new(SessionId::from("summary-session"), SessionSource::Test),
        entries: Vec::new(),
        agent_events: Vec::new(),
        approvals: Vec::new(),
    };
    let records = vec![
        chat_history_record("session-a", "alpha first"),
        chat_history_record("session-b", "beta only"),
        chat_history_record("session-a", "alpha latest token=abc123"),
    ];
    let replays = records
        .into_iter()
        .fold(Vec::<SessionReplay>::new(), |mut replays, record| {
            push_history_record_as_replay_entries(&mut replays, record);
            replays
        });
    let mut replays = replays;
    replays.push(replay);

    let summaries =
        super::super::history::chat_history_session_summaries_from_session_replays(&replays, 10);

    assert_eq!(summaries.len(), 2);
    assert_eq!(summaries[0].session_id, "session-a");
    assert_eq!(summaries[0].turns, 2);
    assert_eq!(summaries[0].agents, vec!["build"]);
    assert!(summaries[0].last_user_message.contains("[REDACTED_SECRET]"));
    assert!(!summaries[0].last_user_message.contains("abc123"));
    assert_eq!(summaries[1].session_id, "session-b");
    assert_eq!(summaries[1].turns, 1);

    let limited =
        super::super::history::chat_history_session_summaries_from_session_replays(&replays, 1);
    assert_eq!(limited.len(), 1);
    assert_eq!(limited[0].session_id, "session-a");
    assert!(
        super::super::history::chat_history_session_summaries_from_session_replays(&replays, 0)
            .is_empty()
    );
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
            "I want you to only inspect docs in this turn. 我希望你这次不要跑测试。"
        )
        .is_empty(),
        "short-lived instructions belong in working memory or candidates, not core relationship memory"
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

#[tokio::test]
async fn chat_context_uses_projection_and_working_memory_without_task_memory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    let memory =
        ikaros_memory::LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    ikaros_memory::MemoryStore::append(
        &memory,
        ikaros_memory::MemoryRecord::new(
            ikaros_memory::MemoryKind::User,
            "default",
            "User preference: concise status",
        )
        .expect("user memory"),
    )
    .expect("append user");
    ikaros_memory::MemoryStore::append(
        &memory,
        ikaros_memory::MemoryRecord::new(
            ikaros_memory::MemoryKind::Project,
            "repo",
            "Working convention: memory and RAG stay separate",
        )
        .expect("project memory"),
    )
    .expect("append project");
    ikaros_memory::MemoryStore::append(
        &memory,
        ikaros_memory::MemoryRecord::new(
            ikaros_memory::MemoryKind::Task,
            "chat-session",
            "Turn summary\nuser: one-off task",
        )
        .expect("task memory")
        .with_tags(vec!["turn-summary".into()]),
    )
    .expect("append task");
    ikaros_memory::JsonlWorkingMemoryStore::new(temp.path().join("memory"))
        .append(
            ikaros_memory::WorkingMemoryRecord::new(
                "chat-session",
                ikaros_memory::MemoryKind::Task,
                "chat-session",
                "Current task goal: keep runtime context local-first",
                Some(24),
            )
            .expect("working memory"),
        )
        .expect("append working");

    let registry = ikaros_skills::builtin_registry(ikaros_skills::SkillEnvironment {
        workspace_root: workspace.clone(),
        memory_store: memory,
        rag_index: ikaros_rag::LocalRagStore::new(temp.path().join("rag"), "jsonl").expect("rag"),
        rag_config: RagConfig {
            embedding_provider: "hash".into(),
            ..RagConfig::default()
        },
        rag_provider: ikaros_core::RemoteProviderConfig::default(),
        persona_path: temp.path().join("persona"),
        skills_dir: temp.path().join("skills"),
        voice_tts: ikaros_voice::VoiceProviderConfig::mock_tts(),
        voice_tts_provider: ikaros_core::RemoteProviderConfig::default(),
        voice_asr: ikaros_voice::VoiceProviderConfig::mock_asr(),
        voice_asr_provider: ikaros_core::RemoteProviderConfig::default(),
        web_search_provider: ikaros_core::RemoteProviderConfig::default(),
        coding_session: None,
    });
    let execution = ikaros_harness::ExecutionSession::new(&workspace, &audit);
    let mut profile = AgentProfile::build();
    profile.rag_context = false;
    let agent = ResolvedAgentProfile {
        name: "build".into(),
        profile,
    };

    let context = build_chat_context(
        "continue",
        &agent,
        &execution,
        &registry,
        &ChatRunOptions {
            session_id: Some("chat-session".into()),
            scope: Some("repo".into()),
            memory_limit: 5,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("context");

    let projection = context.memory_projection.join("\n");
    let working = context.working_memory.join("\n");
    let retrieved = context.retrieved_memory.join("\n");
    assert!(projection.contains("concise status"));
    assert!(projection.contains("memory and RAG stay separate"));
    assert!(working.contains("Current task goal"));
    assert!(
        retrieved.is_empty(),
        "long-term memory search should be opt-in; default chat context uses projection and working memory"
    );

    let searched_context = build_chat_context(
        "memory and RAG",
        &agent,
        &execution,
        &registry,
        &ChatRunOptions {
            session_id: Some("chat-session".into()),
            scope: Some("repo".into()),
            memory_limit: 5,
            memory_search_limit: 2,
            ..ChatRunOptions::default()
        },
    )
    .await
    .expect("searched context");
    let retrieved = searched_context.retrieved_memory.join("\n");
    assert!(retrieved.contains("[Project/repo] Working convention: memory and RAG stay separate"));
    assert!(!retrieved.contains("one-off task"));
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
        persona_path: temp.path().join("persona"),
        skills_dir: temp.path().join("skills"),
        voice_tts: ikaros_voice::VoiceProviderConfig::mock_tts(),
        voice_tts_provider: ikaros_core::RemoteProviderConfig::default(),
        voice_asr: ikaros_voice::VoiceProviderConfig::mock_asr(),
        voice_asr_provider: ikaros_core::RemoteProviderConfig::default(),
        web_search_provider: ikaros_core::RemoteProviderConfig::default(),
        coding_session: None,
    });

    assert!(!context_lookup_is_safe_read(&registry, "rag_search"));
}
