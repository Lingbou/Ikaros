// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn memory_skills_run_through_harness_and_reject_secret_updates() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory = LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    let env = SkillEnvironment {
        memory_store: memory.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let appended = session
        .execute_skill(
            &registry,
            "memory_append",
            json!({"kind": "project", "scope": "repo", "content": "remember local-first"}),
        )
        .await
        .expect("append");
    assert!(appended.ok);
    assert_eq!(memory.list(MemoryQuery::default()).expect("list").len(), 1);

    let record = memory
        .append(MemoryRecord::new(MemoryKind::Project, "repo", "old memory").expect("record"))
        .expect("append");
    let record_id = record.id.clone();
    let updated = session
        .execute_skill(
            &registry,
            "memory_update",
            json!({"id": record_id.clone(), "content": "new memory", "tags": ["edited"]}),
        )
        .await
        .expect("update");
    assert!(updated.ok);
    assert_eq!(updated.output["updated"]["content"], json!("new memory"));
    assert_eq!(
        updated.output["change_report"],
        json!({
            "id": record_id,
            "found": true,
            "content_changed": true,
            "tags_changed": true,
            "changed_fields": ["content", "tags"],
            "before": {
                "content": "old memory",
                "tags": []
            },
            "after": {
                "content": "new memory",
                "tags": ["edited"]
            }
        })
    );

    let rejected = session
        .execute_skill(
            &registry,
            "memory_update",
            json!({"id": record_id.clone(), "content": "token=abc123"}),
        )
        .await
        .expect_err("secret update rejected");
    assert!(rejected.to_string().contains("secret-like"));

    let deleted = session
        .execute_skill(&registry, "memory_delete", json!({"id": record_id}))
        .await
        .expect("delete");
    assert!(deleted.ok);
    assert_eq!(deleted.output["records_deleted"], json!(1));
}

#[tokio::test]
async fn memory_candidate_create_skill_writes_pending_inbox_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory_dir = temp.path().join("memory");
    let memory = LocalMemoryStore::new(&memory_dir, "jsonl").expect("memory");
    let env = SkillEnvironment {
        memory_store: memory.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let created = session
        .execute_skill(
            &registry,
            "memory_candidate_create",
            json!({
                "kind": "relationship",
                "scope": "default",
                "content": "User preference: concise updates",
                "reason": "preference_pattern",
                "confidence": 0.7,
                "tags": ["relationship", "chat-learned"],
                "source_ref": {
                    "type": "session_turn",
                    "data": {"session_id": "chat-1", "turn_id": "turn-1"}
                }
            }),
        )
        .await
        .expect("create candidate");

    assert!(created.ok);
    assert_eq!(created.output["created"], json!(true));
    assert!(
        memory
            .list(MemoryQuery::default())
            .expect("core memory")
            .is_empty(),
        "candidate creation must not promote into core memory"
    );

    let candidates = JsonlMemoryCandidateStore::new(&memory_dir)
        .list(MemoryCandidateQuery {
            status: Some(MemoryCandidateStatus::Pending),
            ..MemoryCandidateQuery::default()
        })
        .expect("pending candidates");
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].content, "User preference: concise updates");
    assert_eq!(
        candidates[0].source_ref,
        Some(MemoryRef::SessionTurn {
            session_id: "chat-1".into(),
            turn_id: Some("turn-1".into())
        })
    );
    let journal_entries = JsonlMemoryJournal::new(&memory_dir)
        .list()
        .expect("candidate journal");
    assert!(journal_entries.iter().any(|entry| {
        entry.action == MemoryJournalAction::CandidateCreated
            && entry.memory_id.as_deref() == Some(&candidates[0].id)
            && entry.kind == Some(MemoryKind::Relationship)
            && entry.scope.as_deref() == Some("default")
            && entry.source_ref == candidates[0].source_ref
    }));

    let duplicate = session
        .execute_skill(
            &registry,
            "memory_candidate_create",
            json!({
                "kind": "relationship",
                "scope": "default",
                "content": "User preference: concise updates",
                "reason": "preference_pattern"
            }),
        )
        .await
        .expect("duplicate candidate");
    assert!(duplicate.ok);
    assert_eq!(duplicate.output["created"], json!(false));
    assert_eq!(duplicate.output["id"], json!(candidates[0].id));
    let journal_entries = JsonlMemoryJournal::new(&memory_dir)
        .list()
        .expect("candidate journal after duplicate");
    assert_eq!(
        journal_entries
            .iter()
            .filter(|entry| entry.action == MemoryJournalAction::CandidateCreated)
            .count(),
        1,
        "duplicate candidates must not append another created journal entry"
    );
}

#[tokio::test]
async fn memory_delete_with_kind_does_not_delete_other_kinds_by_id() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory = LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    let env = SkillEnvironment {
        memory_store: memory.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let project = memory
        .append(MemoryRecord::new(MemoryKind::Project, "repo", "project note").expect("record"))
        .expect("append project");

    let deleted = session
        .execute_skill(
            &registry,
            "memory_delete",
            json!({"id": project.id, "kind": "relationship"}),
        )
        .await
        .expect("kind guarded delete");

    assert!(deleted.ok);
    assert_eq!(deleted.output["records_deleted"], json!(0));
    assert_eq!(memory.list(MemoryQuery::default()).expect("list").len(), 1);
}

#[tokio::test]
async fn memory_delete_with_kind_finds_records_beyond_default_search_limit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory = LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    let env = SkillEnvironment {
        memory_store: memory.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let mut target =
        MemoryRecord::new(MemoryKind::Project, "repo", "old project note").expect("record");
    target.created_at = "2000-01-01T00:00:00Z".into();
    let target = memory.append(target).expect("append target");
    for index in 0..25 {
        let mut record = MemoryRecord::new(
            MemoryKind::Project,
            "repo",
            format!("new project note {index}"),
        )
        .expect("record");
        record.created_at = format!("2099-01-01T00:00:{index:02}Z");
        memory.append(record).expect("append newer");
    }

    let deleted = session
        .execute_skill(
            &registry,
            "memory_delete",
            json!({"id": target.id, "kind": "project"}),
        )
        .await
        .expect("kind guarded delete");

    assert!(deleted.ok);
    assert_eq!(deleted.output["records_deleted"], json!(1));
    assert!(
        memory
            .list(MemoryQuery {
                kind: Some(MemoryKind::Project),
                ..MemoryQuery::default()
            })
            .expect("list")
            .iter()
            .all(|record| record.content != "old project note")
    );
}

#[tokio::test]
async fn memory_projection_skill_renders_core_memory_without_task_summaries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory = LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    let env = SkillEnvironment {
        memory_store: memory.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    memory
        .append(
            MemoryRecord::new(MemoryKind::User, "default", "User preference: concise")
                .expect("record"),
        )
        .expect("append user");
    memory
        .append(
            MemoryRecord::new(
                MemoryKind::Project,
                "repo",
                "Working convention: local-first memory",
            )
            .expect("record"),
        )
        .expect("append project");
    memory
        .append(
            MemoryRecord::new(
                MemoryKind::Task,
                "chat-session",
                "Turn summary\nuser: do this once",
            )
            .expect("record")
            .with_tags(vec!["turn-summary".into()]),
        )
        .expect("append task");

    let projection = session
        .execute_skill(
            &registry,
            "memory_projection",
            json!({"user_scope": "default", "project_scope": "repo"}),
        )
        .await
        .expect("projection");

    assert!(projection.ok);
    assert!(
        projection.output["user"]
            .as_str()
            .expect("user")
            .contains("concise")
    );
    assert!(
        projection.output["project"]
            .as_str()
            .expect("project")
            .contains("local-first memory")
    );
    assert!(
        !projection.output.to_string().contains("do this once"),
        "projection must not expose ordinary episode summaries as core memory"
    );
}

#[tokio::test]
async fn working_memory_list_skill_reads_session_scratchpad() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let memory = LocalMemoryStore::new(temp.path().join("memory"), "jsonl").expect("memory");
    let working = JsonlWorkingMemoryStore::new(temp.path().join("memory"));
    working
        .append(
            WorkingMemoryRecord::new(
                "session-1",
                MemoryKind::Task,
                "session-1",
                "Current task goal: finish memory projection",
                Some(24),
            )
            .expect("working memory")
            .with_source_ref(MemoryRef::SessionTurn {
                session_id: "session-1".into(),
                turn_id: Some("turn-1".into()),
            })
            .expect("source ref"),
        )
        .expect("append working memory");
    let env = SkillEnvironment {
        memory_store: memory,
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(
            &registry,
            "working_memory_list",
            json!({"session_id": "session-1", "limit": 5}),
        )
        .await
        .expect("working memory list");

    assert!(result.ok);
    let records = result.output.as_array().expect("records");
    assert_eq!(records.len(), 1);
    assert!(
        records[0]["content"]
            .as_str()
            .expect("content")
            .contains("memory projection")
    );
}
