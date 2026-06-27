// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn chat_prompt_builder_returns_observable_sections() {
    let context = ContextBuilder::new()
        .persona_context("Persona token=abc123")
        .relationship_context(vec!["Relationship prefers concise updates".into()])
        .reference_context(vec!["Reference src/lib.rs".into()])
        .chat_history_context(vec!["Earlier user asked for a quiet status".into()])
        .memory_projection_context(vec!["Projection says: concise updates".into()])
        .working_memory_context(vec!["Working memory sk-not-real".into()])
        .retrieved_memory_context(vec!["Retrieved memory from search".into()])
        .rag_context(vec!["RAG safe citation".into()])
        .context_continuation_prompt(Some(
            "Compacted sections: history. Do not invent omitted details.".into(),
        ))
        .build();

    let report = build_chat_system_prompt(&context, &HeuristicTokenEstimator);

    assert_eq!(report.prompt, render_chat_system_prompt(&context));
    assert!(
        report
            .sections
            .iter()
            .any(|section| section.kind == PromptSectionKind::Persona
                && section.source == PromptSourceKind::Persona
                && section.priority == 100)
    );
    assert!(
        report
            .sections
            .iter()
            .any(|section| section.kind == PromptSectionKind::Policy
                && section.source == PromptSourceKind::Runtime)
    );
    let tool_guidance = report
        .sections
        .iter()
        .find(|section| {
            section.kind == PromptSectionKind::ToolGuidance
                && section.source == PromptSourceKind::Tooling
        })
        .expect("tool guidance section");
    assert!(tool_guidance.content.contains("no direct tools"));
    assert!(!tool_guidance.content.contains("tool_search"));
    assert!(!tool_guidance.content.contains("tool_describe"));
    assert!(!tool_guidance.content.contains("tool_call"));
    let working_memory = report
        .sections
        .iter()
        .find(|section| section.kind == PromptSectionKind::WorkingMemory)
        .expect("working memory section");
    assert_eq!(working_memory.redaction, PromptRedactionState::Redacted);
    assert!(!working_memory.content.contains("sk-not-real"));
    assert!(working_memory.content.contains("[REDACTED_SECRET]"));
    assert!(
        report
            .sections
            .iter()
            .all(|section| section.estimated_tokens > 0)
    );
    assert!(report.estimated_tokens >= report.sections.len());
}

#[test]
fn single_call_chat_request_splits_prompt_cache_stable_prefix_from_dynamic_context() {
    let runtime_context = ContextBuilder::new()
        .persona_context("stable persona")
        .chat_history_context(vec!["dynamic history token=abc123".into()])
        .retrieved_memory_context(vec!["dynamic memory".into()])
        .build();
    let report = build_chat_system_prompt(&runtime_context, &HeuristicTokenEstimator);

    let messages = super::super::turn::model_messages_for_single_call(
        &report.system_messages_for_prompt_cache(),
        "hello sk-user",
    );

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, "system");
    assert!(messages[0].content.contains("stable persona"));
    assert!(messages[0].content.contains("Policy"));
    assert!(!messages[0].content.contains("dynamic history"));
    assert!(!messages[0].content.contains("dynamic memory"));
    assert_eq!(messages[1].role, "system");
    assert!(messages[1].content.contains("dynamic history"));
    assert!(messages[1].content.contains("dynamic memory"));
    assert!(!messages[1].content.contains("abc123"));
    assert!(messages[1].content.contains("[REDACTED_SECRET]"));
    assert_eq!(messages[2].role, "user");
    assert!(!messages[2].content.contains("sk-user"));
    assert!(messages[2].content.contains("[REDACTED_SECRET]"));
}

#[tokio::test]
async fn agent_loop_chat_request_splits_prompt_cache_stable_prefix_from_dynamic_context() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let audit = temp.path().join("audit");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::write(
        workspace.join("notes.md"),
        "alpha reference line token=abc123\nbeta reference line\n",
    )
    .expect("write reference");
    let execution = ikaros_harness::ExecutionSession::new(&workspace, &audit);
    let registry = ikaros_harness::SkillRegistry::new();
    let persona = PersonaLoader::parse(PersonaLoader::default_markdown()).expect("persona");
    let mut profile = AgentProfile::build();
    profile.memory_context = false;
    profile.rag_context = false;
    let agent = ResolvedAgentProfile {
        name: "build".into(),
        profile,
    };
    let provider = RecordingMessagesProvider::default();

    run_chat_turn_with_events(
        "answer using @file:notes.md:1-1",
        &persona,
        &provider,
        &agent,
        &execution,
        &registry,
        ChatTurnEventOptions {
            options: &ChatRunOptions {
                agent_loop: true,
                stream: false,
                relationship_learning: false,
                session_id: Some("prompt-cache-agent-loop-session".into()),
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
    .expect("agent-loop chat turn");

    let messages = provider.messages.lock().expect("messages").clone();
    let system_messages = messages
        .iter()
        .filter(|message| message.role == "system")
        .collect::<Vec<_>>();
    assert!(
        system_messages.len() >= 2,
        "agent-loop chat should keep dynamic context out of the cache-stable system prefix: {messages:#?}"
    );
    assert!(system_messages[0].content.contains("Agent profile: build"));
    assert!(system_messages[0].content.contains("Tool-call protocol"));
    assert!(!system_messages[0].content.contains("alpha reference line"));
    assert!(
        system_messages[1..]
            .iter()
            .any(|message| message.content.contains("alpha reference line"))
    );
    assert!(
        !messages
            .iter()
            .any(|message| message.content.contains("abc123"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.content.contains("[REDACTED_SECRET]"))
    );
}
