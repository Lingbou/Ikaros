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
