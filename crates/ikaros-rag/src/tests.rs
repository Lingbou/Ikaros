// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::symlink;

#[test]
fn ingests_and_searches_local_files() {
    let temp = tempfile::tempdir().expect("tempdir");
    let doc = temp.path().join("doc.md");
    fs::write(&doc, "# Ikaros\nlocal-first RAG with citation metadata").expect("write");
    let index = LocalRagIndex::new(temp.path().join("rag"));
    let report = index
        .ingest_path(&doc, IngestOptions::default())
        .expect("ingest");
    assert_eq!(report.files_indexed, 1);
    let hits = index
        .search(RagQuery {
            query: "citation".into(),
            top_k: 3,
            scope: Some("project".into()),
        })
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].citation.line_start, 1);
}

#[test]
fn redacts_secret_like_chunks_before_indexing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let doc = temp.path().join("secret.md");
    fs::write(&doc, "key sk-test").expect("write");
    let index = LocalRagIndex::new(temp.path().join("rag"));
    index
        .ingest_path(&doc, IngestOptions::default())
        .expect("ingest");
    let raw = fs::read_to_string(index.path()).expect("index");
    assert!(!raw.contains("sk-test"));
    assert!(raw.contains("[REDACTED_SECRET]"));
}

#[test]
fn jsonl_ingest_persists_hash_embeddings_for_redacted_chunks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let doc = temp.path().join("doc.md");
    fs::write(&doc, "alpha beta beta").expect("write");
    let index = LocalRagIndex::new(temp.path().join("rag"));
    index
        .ingest_path_with_embedding(&doc, IngestOptions::default(), &HashEmbeddingProvider)
        .expect("ingest");
    let chunks = index.read_all().expect("chunks");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].embedding_provider.as_deref(), Some("hash"));
    assert!(
        chunks[0]
            .embedding
            .as_ref()
            .is_some_and(|values| values.len() == 32)
    );
    let hits = index
        .search_with_embedding(
            RagQuery {
                query: "beta".into(),
                top_k: 1,
                scope: Some("project".into()),
            },
            &HashEmbeddingProvider,
        )
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert!(hits[0].score > 1.0);
}

#[test]
fn content_based_ingest_works_for_jsonl_and_sqlite_backends() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sources = vec![IngestSourceFile {
        source_path: temp.path().join("doc.md"),
        content: "env sourced searchable text".into(),
        modified_at: Some("2026-06-17T00:00:00Z".into()),
    }];

    let jsonl = LocalRagIndex::new(temp.path().join("jsonl"));
    let sqlite = SqliteRagIndex::new(temp.path().join("sqlite"));
    let jsonl_report = jsonl
        .ingest_sources_with_embedding(
            sources.clone(),
            IngestOptions::default(),
            &HashEmbeddingProvider,
        )
        .expect("jsonl source ingest");
    let sqlite_report = sqlite
        .ingest_sources_with_embedding(sources, IngestOptions::default(), &HashEmbeddingProvider)
        .expect("sqlite source ingest");

    assert_eq!(jsonl_report.files_indexed, 1);
    assert_eq!(sqlite_report.files_indexed, 1);
    assert_eq!(
        jsonl
            .search(RagQuery {
                query: "searchable".into(),
                top_k: 1,
                scope: Some("project".into()),
            })
            .expect("jsonl search")
            .len(),
        1
    );
    assert_eq!(
        sqlite
            .search(RagQuery {
                query: "searchable".into(),
                top_k: 1,
                scope: Some("project".into()),
            })
            .expect("sqlite search")
            .len(),
        1
    );
}

#[test]
fn jsonl_reingest_reports_chunks_written_this_run() {
    let temp = tempfile::tempdir().expect("tempdir");
    let doc = temp.path().join("doc.md");
    fs::write(&doc, "one\ntwo").expect("write");
    let index = LocalRagIndex::new(temp.path().join("rag"));
    index
        .ingest_path(
            &doc,
            IngestOptions {
                max_chunk_lines: 1,
                ..IngestOptions::default()
            },
        )
        .expect("ingest");
    let report = index
        .ingest_path(
            &doc,
            IngestOptions {
                max_chunk_lines: 1,
                ..IngestOptions::default()
            },
        )
        .expect("reingest");
    assert_eq!(report.chunks_indexed, 2);
}

#[test]
fn jsonl_delete_path_removes_file_chunks() {
    let temp = tempfile::tempdir().expect("tempdir");
    let keep = temp.path().join("keep.md");
    let drop = temp.path().join("drop.md");
    fs::write(&keep, "keep alpha").expect("write keep");
    fs::write(&drop, "drop beta").expect("write drop");
    let index = LocalRagIndex::new(temp.path().join("rag"));
    index
        .ingest_path(temp.path(), IngestOptions::default())
        .expect("ingest");

    let deleted = index.delete_path(&drop).expect("delete path");
    assert_eq!(deleted, 1);
    assert!(
        index
            .search(RagQuery {
                query: "beta".into(),
                top_k: 5,
                scope: Some("project".into()),
            })
            .expect("search")
            .is_empty()
    );
    assert_eq!(
        index
            .search(RagQuery {
                query: "alpha".into(),
                top_k: 5,
                scope: Some("project".into()),
            })
            .expect("search")
            .len(),
        1
    );
}

#[cfg(unix)]
#[test]
fn jsonl_ingest_skips_symlinked_directories() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&outside).expect("outside");
    fs::write(workspace.join("local.md"), "local searchable").expect("local");
    fs::write(outside.join("external.md"), "external searchable").expect("external");
    symlink(&outside, workspace.join("linked")).expect("symlink");
    let index = LocalRagIndex::new(temp.path().join("rag"));

    let report = index
        .ingest_path(&workspace, IngestOptions::default())
        .expect("ingest");

    assert_eq!(report.files_indexed, 1);
    assert_eq!(
        index
            .search(RagQuery {
                query: "local".into(),
                top_k: 5,
                scope: Some("project".into()),
            })
            .expect("local search")
            .len(),
        1
    );
    assert!(
        index
            .search(RagQuery {
                query: "external".into(),
                top_k: 5,
                scope: Some("project".into()),
            })
            .expect("external search")
            .is_empty()
    );
}

#[cfg(unix)]
#[test]
fn jsonl_ingest_rejects_explicit_symlink_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let outside = temp.path().join("outside.md");
    fs::write(&outside, "external searchable").expect("outside");
    let link = temp.path().join("link.md");
    symlink(&outside, &link).expect("symlink");
    let index = LocalRagIndex::new(temp.path().join("rag"));

    let error = index
        .ingest_path(&link, IngestOptions::default())
        .expect_err("symlink path rejected");

    assert!(error.to_string().contains("symlink"));
}

#[test]
fn jsonl_stale_files_include_deleted_sources() {
    let temp = tempfile::tempdir().expect("tempdir");
    let doc = temp.path().join("doc.md");
    fs::write(&doc, "stale candidate").expect("write");
    let index = LocalRagIndex::new(temp.path().join("rag"));
    index
        .ingest_path(&doc, IngestOptions::default())
        .expect("ingest");
    fs::remove_file(&doc).expect("remove");

    let stale = index.stale_files().expect("stale");
    assert_eq!(stale, vec![doc]);
}

#[test]
fn jsonl_reingest_preserves_other_scopes_for_same_path() {
    let temp = tempfile::tempdir().expect("tempdir");
    let doc = temp.path().join("doc.md");
    fs::write(&doc, "scope preserve").expect("write");
    let index = LocalRagIndex::new(temp.path().join("rag"));
    for scope in ["project-a", "project-b", "project-b"] {
        index
            .ingest_path(
                &doc,
                IngestOptions {
                    scope: scope.into(),
                    ..IngestOptions::default()
                },
            )
            .expect("ingest");
    }
    assert_eq!(
        index
            .search(RagQuery {
                query: "preserve".into(),
                top_k: 5,
                scope: Some("project-a".into()),
            })
            .expect("search a")
            .len(),
        1
    );
    assert_eq!(
        index
            .search(RagQuery {
                query: "preserve".into(),
                top_k: 5,
                scope: Some("project-b".into()),
            })
            .expect("search b")
            .len(),
        1
    );
}

#[test]
fn sparse_embedding_provider_normalizes_vectors() {
    let embedding = SparseEmbeddingProvider
        .embed("alpha beta beta")
        .expect("embedding");
    let norm = embedding
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    assert!((norm - 1.0).abs() < 0.001);
}

#[test]
fn rag_core_rejects_remote_embedding_provider_without_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = LocalRagStore::new(temp.path().join("rag"), "jsonl").expect("rag store");
    for provider in ["openai-compatible", "ollama"] {
        let err = store
            .search_with_embedding_provider(
                RagQuery {
                    query: "remote probe".into(),
                    top_k: 1,
                    scope: None,
                },
                provider,
            )
            .expect_err("remote embedding providers must not be built inside ikaros-rag");
        let err = err.to_string();
        assert!(err.contains("ExecutionEnv"), "{provider}: {err}");
        assert!(!err.contains("sk-test-secret"));
    }
}

#[test]
fn ingests_and_searches_sqlite_index() {
    let temp = tempfile::tempdir().expect("tempdir");
    let doc = temp.path().join("doc.md");
    fs::write(&doc, "# Harness\npolicy audit approval").expect("write");
    let index = SqliteRagIndex::new(temp.path().join("rag"));
    let report = index
        .ingest_path(&doc, IngestOptions::default())
        .expect("ingest");
    assert_eq!(report.files_indexed, 1);
    let hits = index
        .search(RagQuery {
            query: "approval".into(),
            top_k: 2,
            scope: Some("project".into()),
        })
        .expect("search");
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].citation.line_start, 1);
}

#[test]
fn sqlite_ingest_persists_hash_embeddings() {
    let temp = tempfile::tempdir().expect("tempdir");
    let doc = temp.path().join("doc.md");
    fs::write(&doc, "embedding backed retrieval").expect("write");
    let index = SqliteRagIndex::new(temp.path().join("rag"));
    index
        .ingest_path_with_embedding(&doc, IngestOptions::default(), &HashEmbeddingProvider)
        .expect("ingest");
    let chunks = index.read_all().expect("chunks");
    assert_eq!(chunks.len(), 1);
    assert_eq!(chunks[0].embedding_provider.as_deref(), Some("hash"));
    assert!(
        chunks[0]
            .embedding
            .as_ref()
            .is_some_and(|values| values.len() == 32)
    );
}

#[test]
fn sqlite_delete_scope_removes_only_matching_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    let doc = temp.path().join("doc.md");
    fs::write(&doc, "scope cleanup").expect("write");
    let index = SqliteRagIndex::new(temp.path().join("rag"));
    index
        .ingest_path(
            &doc,
            IngestOptions {
                scope: "project-a".into(),
                ..IngestOptions::default()
            },
        )
        .expect("ingest a");
    index
        .ingest_path(
            &doc,
            IngestOptions {
                scope: "project-b".into(),
                ..IngestOptions::default()
            },
        )
        .expect("ingest b");

    assert_eq!(index.delete_scope("project-a").expect("delete scope"), 1);
    assert!(
        index
            .search(RagQuery {
                query: "cleanup".into(),
                top_k: 5,
                scope: Some("project-a".into()),
            })
            .expect("search a")
            .is_empty()
    );
    assert_eq!(
        index
            .search(RagQuery {
                query: "cleanup".into(),
                top_k: 5,
                scope: Some("project-b".into()),
            })
            .expect("search b")
            .len(),
        1
    );
}
