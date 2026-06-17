// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn appends_and_searches_local_jsonl_memory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = JsonlMemoryStore::new(temp.path());
    let record = store
        .append(
            MemoryRecord::new(MemoryKind::Project, "repo", "local-first RAG decision")
                .expect("record")
                .with_tags(vec!["rag".into()]),
        )
        .expect("append");
    let found = store
        .search(MemoryQuery {
            text: Some("rag".into()),
            ..MemoryQuery::default()
        })
        .expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].scope, "repo");

    let updated = store
        .update(
            &record.id,
            Some("local-first RAG and memory decision".into()),
            Some(vec!["rag".into(), "memory".into()]),
        )
        .expect("update")
        .expect("updated");
    assert!(updated.updated_at.is_some());
    assert!(updated.tags.contains(&"memory".into()));
    assert!(
        store
            .update(&record.id, Some("token=abc123".into()), None)
            .expect_err("secret update rejected")
            .to_string()
            .contains("secret-like")
    );
    assert!(store.delete_by_id(&record.id).expect("delete"));
    assert!(
        store
            .search(MemoryQuery {
                text: Some("memory".into()),
                ..MemoryQuery::default()
            })
            .expect("search after delete")
            .is_empty()
    );
}

#[test]
fn rejects_secret_like_memory_entries() {
    let err = MemoryRecord::new(MemoryKind::User, "default", "token=abc123")
        .expect_err("secret rejected");
    assert!(err.to_string().contains("secret-like"));
}

#[test]
fn memory_journal_records_policy_actions() {
    let temp = tempfile::tempdir().expect("tempdir");
    let journal = JsonlMemoryJournal::new(temp.path());
    let score = MemoryScore {
        recency: 0.8,
        relevance: 0.9,
        frequency: 0.8,
        source_strength: 0.9,
        confidence: 0.9,
        sensitivity: 0.0,
    };
    assert!(score.combined() > MemoryPolicy::default().promote_threshold);

    let entry = MemoryJournalEntry::new(MemoryJournalAction::Promote, "stable user preference")
        .expect("entry")
        .with_memory("memory-1", MemoryKind::User, "default")
        .expect("memory")
        .with_score(score)
        .with_source_ref(MemoryRef::SessionTurn {
            session_id: "session-1".into(),
            turn_id: Some("turn-1".into()),
        })
        .expect("source ref");
    journal.append(entry).expect("append");

    let entries = journal.list().expect("list");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, MemoryJournalAction::Promote);
    assert_eq!(entries[0].scope.as_deref(), Some("default"));
    assert!(entries[0].score.expect("score").combined() > 0.0);
}

#[test]
fn memory_policy_engine_scores_promotes_demotes_and_selects_quota_victims() {
    let policy = MemoryPolicy {
        promote_threshold: 0.55,
        demote_threshold: 0.45,
        forget_threshold: 0.10,
        max_records_per_scope: 2,
    };
    let engine = MemoryPolicyEngine::new(policy);
    let promoted = MemoryRecord::new(
        MemoryKind::Relationship,
        "user",
        "User preference: concise updates",
    )
    .expect("promoted")
    .with_tags(vec!["relationship".into(), "chat-learned".into()])
    .with_source("manual");
    let demoted = MemoryRecord::new(MemoryKind::Task, "user", "old")
        .expect("demoted")
        .with_tags(Vec::new());
    let quota_victim = MemoryRecord::new(MemoryKind::Task, "user", "stale")
        .expect("quota")
        .with_tags(Vec::new());
    let scope_records = vec![promoted.clone(), demoted.clone(), quota_victim.clone()];

    let promote = engine
        .classify_record(&promoted, &scope_records)
        .expect("promote decision");
    assert_eq!(promote.action, MemoryJournalAction::Promote);

    let demote = engine
        .classify_record(&demoted, &scope_records)
        .expect("demote decision");
    assert_eq!(demote.action, MemoryJournalAction::Demote);

    let victims = engine.quota_victims(&scope_records);
    assert_eq!(victims.len(), 1);
    assert!(victims[0].1.combined() <= promote.score.combined());
}

#[test]
fn rejects_secret_like_memory_metadata() {
    let err = MemoryRecord::new(MemoryKind::User, "scope token=abc123", "plain content")
        .expect_err("secret scope rejected");
    assert!(err.to_string().contains("memory scope"));

    let temp = tempfile::tempdir().expect("tempdir");
    let jsonl = JsonlMemoryStore::new(temp.path().join("jsonl"));
    let err = jsonl
        .append(
            MemoryRecord::new(MemoryKind::Project, "repo", "plain content")
                .expect("record")
                .with_tags(vec!["token=abc123".into()]),
        )
        .expect_err("secret tag rejected");
    assert!(err.to_string().contains("memory tag"));
    let err = jsonl
        .append(
            MemoryRecord::new(MemoryKind::Project, "repo", "plain content")
                .expect("record")
                .with_source("password=hunter2"),
        )
        .expect_err("secret source rejected");
    assert!(err.to_string().contains("memory source"));

    let sqlite = SqliteMemoryStore::new(temp.path().join("sqlite"));
    let record = sqlite
        .append(MemoryRecord::new(MemoryKind::Project, "repo", "plain content").expect("record"))
        .expect("append");
    let err = sqlite
        .update(&record.id, None, Some(vec!["api_key=abc123".into()]))
        .expect_err("secret update tag rejected");
    assert!(err.to_string().contains("memory tag"));
}

#[test]
fn appends_and_searches_sqlite_memory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteMemoryStore::new(temp.path());
    store
        .append(
            MemoryRecord::new(MemoryKind::Relationship, "user", "prefers concise updates")
                .expect("record")
                .with_tags(vec!["relationship".into()]),
        )
        .expect("append");
    let found = store
        .search(MemoryQuery {
            kind: Some(MemoryKind::Relationship),
            text: Some("concise".into()),
            ..MemoryQuery::default()
        })
        .expect("search");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].content, "prefers concise updates");
}

#[test]
fn sqlite_deletes_scope_with_optional_kind() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = SqliteMemoryStore::new(temp.path());
    for kind in [MemoryKind::Project, MemoryKind::Relationship] {
        store
            .append(MemoryRecord::new(kind, "shared", "scope cleanup").expect("record"))
            .expect("append");
    }
    assert_eq!(
        store
            .delete_scope(Some(MemoryKind::Project), "shared")
            .expect("delete kind scope"),
        1
    );
    assert_eq!(
        store
            .search(MemoryQuery {
                scope: Some("shared".into()),
                ..MemoryQuery::default()
            })
            .expect("remaining")
            .len(),
        1
    );
    assert_eq!(store.delete_scope(None, "shared").expect("delete scope"), 1);
    assert!(
        store
            .search(MemoryQuery {
                scope: Some("shared".into()),
                ..MemoryQuery::default()
            })
            .expect("empty")
            .is_empty()
    );
}

#[test]
fn provider_registry_reports_local_provider_by_backend() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry =
        MemoryProviderRegistry::from_config(temp.path(), "sqlite", &[]).expect("registry");

    assert_eq!(registry.active_local.id, "local-sqlite");
    assert_eq!(registry.active_local.backend, "sqlite");
    let expected_path = temp.path().join("memory.sqlite");
    assert_eq!(
        registry.active_local.path.as_deref(),
        Some(expected_path.as_path())
    );
    assert!(registry.external.is_empty());
    assert!(registry.issues.is_empty());
}

#[test]
fn local_memory_provider_exposes_lifecycle_hooks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalMemoryStore::new(temp.path(), "jsonl").expect("store");
    MemoryProvider::append(
        &store,
        MemoryRecord::new(MemoryKind::Project, "repo", "runtime boundary").expect("record"),
    )
    .expect("append");

    let start = store
        .turn_start(MemoryTurnStart {
            session_id: Some("s1".into()),
            agent_id: Some("build".into()),
            user_input: "inspect runtime".into(),
        })
        .expect("turn start");
    let records = store
        .prefetch(MemoryPrefetchInput {
            query: MemoryQuery {
                text: Some("runtime".into()),
                ..MemoryQuery::default()
            },
            session_id: Some("s1".into()),
            agent_id: Some("build".into()),
        })
        .expect("prefetch");

    assert_eq!(start.phase, "turn_start");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].content, "runtime boundary");
    assert_eq!(
        store
            .session_switch(MemorySessionSwitch {
                from_session_id: Some("s1".into()),
                to_session_id: Some("s2".into()),
                agent_id: Some("build".into()),
            })
            .expect("session switch")
            .phase,
        "session_switch"
    );
}

#[test]
fn local_memory_provider_sync_turn_writes_source_ref_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalMemoryStore::new(temp.path(), "jsonl").expect("store");

    let report = store
        .sync_turn(MemoryTurnRecord {
            session_id: Some("chat-session".into()),
            turn_id: Some("turn-1".into()),
            agent_id: Some("build".into()),
            user_input: "remember this decision".into(),
            assistant_output: "we chose the session store path".into(),
        })
        .expect("sync turn");

    assert_eq!(report.phase, "sync_turn");
    assert_eq!(report.records_written, 1);
    let records = MemoryStore::search(
        &store,
        MemoryQuery {
            kind: Some(MemoryKind::Task),
            scope: Some("chat-session".into()),
            text: Some("session store".into()),
            limit: Some(10),
        },
    )
    .expect("records");
    assert_eq!(records.len(), 1);
    assert_eq!(
        records[0].source_ref,
        Some(MemoryRef::SessionTurn {
            session_id: "chat-session".into(),
            turn_id: Some("turn-1".into()),
        })
    );
}

#[test]
fn provider_registry_blocks_multiple_active_external_providers() {
    let temp = tempfile::tempdir().expect("tempdir");
    let providers = vec![
        ikaros_core::ExternalMemoryProviderConfig {
            id: "remote-a".into(),
            provider: "plugin".into(),
            enabled: true,
            endpoint: Some("http://127.0.0.1:8787".into()),
            api_key: Some("memory-key-a".into()),
        },
        ikaros_core::ExternalMemoryProviderConfig {
            id: "remote-b".into(),
            provider: "plugin".into(),
            enabled: true,
            endpoint: Some("http://127.0.0.1:8788".into()),
            api_key: Some("memory-key-b".into()),
        },
    ];

    let registry =
        MemoryProviderRegistry::from_config(temp.path(), "jsonl", &providers).expect("registry");

    assert_eq!(registry.active_external_count(), 0);
    assert_eq!(registry.external.len(), 2);
    assert!(registry.external.iter().all(|provider| {
        provider.state == MemoryProviderState::Blocked
            && provider
                .notes
                .iter()
                .any(|note| note.contains("multiple external providers"))
    }));
    assert!(registry.ensure_single_active_external().is_err());
    assert!(registry.issues[0].contains("only one external memory provider"));
}
