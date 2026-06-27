// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::ContextBuilder;
use ikaros_runtime::{
    ChatRunOptions, extract_rag_context, extract_retrieved_memory_context,
    render_chat_system_prompt,
};

#[test]
fn chat_context_extractors_redact_values() {
    let memory = serde_json::json!([
        {
            "kind": "Relationship",
            "scope": "user",
            "content": "Relationship context should be handled separately"
        },
        {
            "kind": "Task",
            "scope": "session",
            "content": "Turn summary should stay in episode history"
        },
        {
            "kind": "Project",
            "scope": "repo",
            "content": "Prefer local RAG and never expose token=abc123"
        }
    ]);
    let retrieved_memory = extract_retrieved_memory_context(&memory, 5);
    assert_eq!(retrieved_memory.len(), 1);
    assert!(retrieved_memory[0].contains("[Project/repo]"));
    assert!(!retrieved_memory[0].contains("Relationship context"));
    assert!(!retrieved_memory[0].contains("Turn summary"));
    assert!(!retrieved_memory[0].contains("abc123"));
    assert!(retrieved_memory[0].contains("[REDACTED_SECRET]"));

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
fn chat_defaults_do_not_auto_inject_rag() {
    assert_eq!(ChatRunOptions::default().rag_top_k, 0);
}

#[test]
fn chat_system_prompt_uses_context_and_redacts() {
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
    let prompt = render_chat_system_prompt(&context);
    assert!(prompt.contains("Local relationship context"));
    assert!(prompt.contains("Relationship prefers concise updates"));
    assert!(prompt.contains("Local reference context"));
    assert!(prompt.contains("Reference src/lib.rs"));
    assert!(prompt.contains("Local chat history context"));
    assert!(prompt.contains("Earlier user asked for a quiet status"));
    assert!(prompt.contains("Accepted memory projection"));
    assert!(prompt.contains("Projection says: concise updates"));
    assert!(prompt.contains("Session working memory"));
    assert!(prompt.contains("Retrieved memory context"));
    assert!(prompt.contains("Retrieved memory from search"));
    assert!(prompt.contains("Local RAG context"));
    assert!(prompt.contains("RAG safe citation"));
    assert!(prompt.contains("Context compression notice"));
    assert!(prompt.contains("Compacted sections: history"));
    assert!(!prompt.contains("abc123"));
    assert!(!prompt.contains("sk-not-real"));
    assert!(prompt.contains("[REDACTED_SECRET]"));
}
