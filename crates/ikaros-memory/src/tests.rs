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
    let mut promoted = MemoryRecord::new(
        MemoryKind::Relationship,
        "user",
        "User preference: concise updates",
    )
    .expect("promoted")
    .with_tags(vec!["relationship".into(), "chat-learned".into()])
    .with_source("manual");
    let mut demoted = MemoryRecord::new(MemoryKind::Task, "user", "old")
        .expect("demoted")
        .with_tags(Vec::new());
    let mut quota_victim = MemoryRecord::new(MemoryKind::Task, "user", "stale")
        .expect("quota")
        .with_tags(Vec::new());
    demoted.created_at = "2026-06-20T00:00:00Z".into();
    quota_victim.created_at = "2026-06-20T00:00:01Z".into();
    promoted.created_at = "2026-06-20T00:00:02Z".into();
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
fn memory_store_supersedes_records_without_deleting_history() {
    let temp = tempfile::tempdir().expect("tempdir");
    let jsonl = JsonlMemoryStore::new(temp.path().join("jsonl"));
    assert_memory_store_supersedes_record(&jsonl);

    let sqlite = SqliteMemoryStore::new(temp.path().join("sqlite"));
    assert_memory_store_supersedes_record(&sqlite);
}

fn assert_memory_store_supersedes_record(store: &dyn MemoryStore) {
    let old = store
        .append(
            MemoryRecord::new(
                MemoryKind::User,
                "default",
                "User prefers verbose status updates",
            )
            .expect("old memory"),
        )
        .expect("append old");
    let replacement = MemoryRecord::new(
        MemoryKind::User,
        "default",
        "User prefers concise status updates",
    )
    .expect("replacement memory");

    let (superseded, active) = store
        .supersede(&old.id, replacement)
        .expect("supersede")
        .expect("superseded");

    assert!(!superseded.active);
    assert_eq!(
        superseded.superseded_by.as_deref(),
        Some(active.id.as_str())
    );
    assert_eq!(superseded.valid_until, active.valid_from);
    assert!(active.active);
    assert_eq!(active.supersedes, vec![old.id.clone()]);

    let records = store
        .list(MemoryQuery {
            scope: Some("default".into()),
            ..MemoryQuery::default()
        })
        .expect("list");
    assert_eq!(records.len(), 2);
    assert!(records.iter().any(|record| record.id == old.id));

    let projection = ProjectionRenderer::default()
        .render(MemoryProjectionInput {
            user_scope: "default".into(),
            project_scope: None,
            perspective: None,
            records,
        })
        .expect("projection");
    assert!(projection.user.contains("concise status updates"));
    assert!(!projection.user.contains("verbose status updates"));
}

#[test]
fn memory_store_filters_perspective_specific_records() {
    let temp = tempfile::tempdir().expect("tempdir");
    let jsonl = JsonlMemoryStore::new(temp.path().join("jsonl"));
    assert_memory_store_filters_perspective(&jsonl);

    let sqlite = SqliteMemoryStore::new(temp.path().join("sqlite"));
    assert_memory_store_filters_perspective(&sqlite);
}

fn assert_memory_store_filters_perspective(store: &dyn MemoryStore) {
    let alice_view = MemoryPerspective::new("alice", "bob").expect("alice perspective");
    let carol_view = MemoryPerspective::new("carol", "bob").expect("carol perspective");
    store
        .append(
            MemoryRecord::new(MemoryKind::Relationship, "default", "Bob likes pancakes")
                .expect("alice memory")
                .with_perspective(alice_view.clone()),
        )
        .expect("append alice view");
    store
        .append(
            MemoryRecord::new(MemoryKind::Relationship, "default", "Bob prefers waffles")
                .expect("carol memory")
                .with_perspective(carol_view.clone()),
        )
        .expect("append carol view");

    let alice_records = store
        .search(MemoryQuery {
            perspective: Some(alice_view),
            text: Some("Bob".into()),
            ..MemoryQuery::default()
        })
        .expect("search alice view");
    assert_eq!(alice_records.len(), 1);
    assert!(alice_records[0].content.contains("pancakes"));

    let carol_records = store
        .list(MemoryQuery {
            perspective: Some(carol_view),
            ..MemoryQuery::default()
        })
        .expect("list carol view");
    assert_eq!(carol_records.len(), 1);
    assert!(carol_records[0].content.contains("waffles"));
}

#[test]
fn projection_renderer_keeps_perspectives_isolated() {
    let alice_view = MemoryPerspective::new("alice", "bob").expect("alice perspective");
    let carol_view = MemoryPerspective::new("carol", "bob").expect("carol perspective");
    let records = vec![
        MemoryRecord::new(MemoryKind::Relationship, "default", "Bob likes pancakes")
            .expect("alice memory")
            .with_perspective(alice_view.clone()),
        MemoryRecord::new(MemoryKind::Relationship, "default", "Bob prefers waffles")
            .expect("carol memory")
            .with_perspective(carol_view),
    ];

    let projection = ProjectionRenderer::default()
        .render(MemoryProjectionInput {
            user_scope: "default".into(),
            project_scope: None,
            perspective: Some(alice_view),
            records,
        })
        .expect("projection");

    assert!(projection.user.contains("pancakes"));
    assert!(!projection.user.contains("waffles"));
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
fn local_memory_provider_sync_turn_reports_source_ref_without_core_record() {
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
    assert_eq!(
        report.source_ref,
        Some(MemoryRef::SessionTurn {
            session_id: "chat-session".into(),
            turn_id: Some("turn-1".into()),
        })
    );
    assert_eq!(report.records.len(), 1);
    assert_eq!(report.records[0].kind, MemoryKind::Task);
    assert_eq!(report.records[0].scope, "chat-session");
    assert_eq!(report.records[0].source_ref, report.source_ref);
    let records = MemoryStore::search(
        &store,
        MemoryQuery {
            kind: Some(MemoryKind::Task),
            scope: Some("chat-session".into()),
            perspective: None,
            text: Some("session store".into()),
            limit: Some(10),
        },
    )
    .expect("records");
    assert!(records.is_empty());
}

#[test]
fn local_memory_provider_sync_turn_writes_working_memory_not_core_task_memory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalMemoryStore::new(temp.path(), "jsonl").expect("store");

    let report = store
        .sync_turn(MemoryTurnRecord {
            session_id: Some("chat-session".into()),
            turn_id: Some("turn-1".into()),
            agent_id: Some("build".into()),
            user_input: "this is a temporary turn goal".into(),
            assistant_output: "we should keep it only for the current session".into(),
        })
        .expect("sync turn");

    assert_eq!(report.phase, "sync_turn");
    assert_eq!(report.records_written, 1);
    assert!(
        report
            .notes
            .iter()
            .any(|note| note == "working_memory_written")
    );
    let long_term = MemoryStore::search(
        &store,
        MemoryQuery {
            kind: Some(MemoryKind::Task),
            scope: Some("chat-session".into()),
            perspective: None,
            text: Some("temporary turn goal".into()),
            limit: Some(10),
        },
    )
    .expect("long-term search");
    assert!(
        long_term.is_empty(),
        "sync_turn must not promote ordinary turn summaries into core Task memory"
    );

    let working = JsonlWorkingMemoryStore::new(temp.path());
    let scratchpad = working
        .list(WorkingMemoryQuery {
            session_id: Some("chat-session".into()),
            ..WorkingMemoryQuery::default()
        })
        .expect("working memory");
    assert_eq!(scratchpad.len(), 1);
    assert_eq!(scratchpad[0].source_ref, report.source_ref);
    assert!(scratchpad[0].content.contains("temporary turn goal"));
}

#[test]
fn working_memory_prune_expired_removes_only_expired_records() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = JsonlWorkingMemoryStore::new(temp.path());
    let mut expired = WorkingMemoryRecord::new(
        "chat-session",
        MemoryKind::Task,
        "chat-session",
        "expired scratchpad",
        None,
    )
    .expect("expired");
    expired.expires_at = Some("2000-01-01T00:00:00Z".into());
    let active = WorkingMemoryRecord::new(
        "chat-session",
        MemoryKind::Task,
        "chat-session",
        "active scratchpad",
        None,
    )
    .expect("active");

    store.append(expired.clone()).expect("append expired");
    store.append(active.clone()).expect("append active");

    let visible = store
        .list(WorkingMemoryQuery {
            include_expired: false,
            ..WorkingMemoryQuery::default()
        })
        .expect("visible");
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].id, active.id);

    let pruned = store.prune_expired().expect("prune expired");
    assert_eq!(pruned.len(), 1);
    assert_eq!(pruned[0].id, expired.id);

    let all = store
        .list(WorkingMemoryQuery {
            include_expired: true,
            ..WorkingMemoryQuery::default()
        })
        .expect("all");
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].id, active.id);
}

#[test]
fn memory_candidate_store_tracks_pending_accept_and_reject_without_core_writes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let candidates = JsonlMemoryCandidateStore::new(temp.path());

    let first = candidates
        .create(
            MemoryCandidate::new(
                MemoryKind::Relationship,
                "user",
                "User asked Ikaros to remember: never commit unless explicitly requested",
                MemoryCandidateReason::ExplicitRemember,
                0.92,
            )
            .expect("candidate")
            .with_source_ref(MemoryRef::SessionTurn {
                session_id: "session-1".into(),
                turn_id: Some("turn-1".into()),
            })
            .expect("source ref"),
        )
        .expect("create candidate");
    let second = candidates
        .create(
            MemoryCandidate::new(
                MemoryKind::Task,
                "session-1",
                "Temporary PR scope: docs only",
                MemoryCandidateReason::TaskOutcome,
                0.45,
            )
            .expect("candidate"),
        )
        .expect("create candidate");

    assert_eq!(
        candidates
            .list(MemoryCandidateQuery {
                status: Some(MemoryCandidateStatus::Pending),
                ..MemoryCandidateQuery::default()
            })
            .expect("pending")
            .len(),
        2
    );

    let accepted = candidates
        .set_status(
            &first.id,
            MemoryCandidateStatus::Accepted,
            "explicit remember can be promoted",
        )
        .expect("accept")
        .expect("accepted candidate");
    let rejected = candidates
        .set_status(
            &second.id,
            MemoryCandidateStatus::Rejected,
            "temporary task scope stays episode history",
        )
        .expect("reject")
        .expect("rejected candidate");

    assert_eq!(accepted.status, MemoryCandidateStatus::Accepted);
    assert_eq!(rejected.status, MemoryCandidateStatus::Rejected);
    assert!(accepted.reviewed_at.is_some());
    assert_eq!(
        accepted.review_reason.as_deref(),
        Some("explicit remember can be promoted")
    );
}

#[test]
fn projection_renderer_outputs_stable_core_memory_and_omits_turn_summaries() {
    let mut user = MemoryRecord::new(
        MemoryKind::User,
        "default",
        "User preference: concise updates",
    )
    .expect("user memory")
    .with_tags(vec!["policy-promoted".into()]);
    user.confidence = Some(0.95);
    let project = MemoryRecord::new(
        MemoryKind::Project,
        "ikaros",
        "Working convention: memory and RAG stay separate",
    )
    .expect("project memory")
    .with_tags(vec!["policy-promoted".into()]);
    let relationship = MemoryRecord::new(
        MemoryKind::Relationship,
        "default",
        "User asked Ikaros to remember: do not commit without approval",
    )
    .expect("relationship memory")
    .with_tags(vec!["policy-promoted".into()]);
    let task_summary = MemoryRecord::new(
        MemoryKind::Task,
        "chat-session",
        "Turn summary\nuser: temporary request\nassistant: done",
    )
    .expect("task memory")
    .with_tags(vec!["turn-summary".into()]);

    let projection = ProjectionRenderer::default()
        .render(MemoryProjectionInput {
            user_scope: "default".into(),
            project_scope: Some("ikaros".into()),
            perspective: None,
            records: vec![user, project, relationship, task_summary],
        })
        .expect("projection");

    assert!(projection.user.contains("# User"));
    assert!(projection.user.contains("concise updates"));
    assert!(projection.user.contains("do not commit without approval"));
    assert!(projection.project.contains("# Project Memory: ikaros"));
    assert!(projection.project.contains("memory and RAG stay separate"));
    assert!(!projection.general.contains("temporary request"));
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
