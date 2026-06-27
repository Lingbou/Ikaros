// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn registry_blocks_temp_rag_ingest_through_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join(".temp")).expect("mkdir");
    fs::write(workspace.join(".temp/secret.md"), "secret").expect("write");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let result = session
        .execute_skill(&registry, "rag_ingest", json!({"path": ".temp/secret.md"}))
        .await
        .expect("skill");
    assert!(!result.ok);
    assert!(result.summary.contains(".temp"));
}

#[tokio::test]
async fn rag_maintenance_skills_run_through_harness() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let doc = workspace.join("doc.md");
    fs::write(&doc, "cleanup index entry").expect("write");
    let rag = LocalRagStore::new(temp.path().join("rag"), "jsonl").expect("rag");
    let env = SkillEnvironment {
        rag_index: rag.clone(),
        ..test_env(temp.path(), &workspace)
    };
    let registry = builtin_registry(env);
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let ingest = session
        .execute_skill(
            &registry,
            "rag_ingest",
            json!({"path": "doc.md", "scope": "cleanup"}),
        )
        .await
        .expect("ingest");
    assert!(ingest.ok);
    let search = session
        .execute_skill(
            &registry,
            "rag_search",
            json!({"query": "cleanup", "scope": "cleanup"}),
        )
        .await
        .expect("search");
    assert!(search.ok);
    assert_eq!(search.output.as_array().expect("hits").len(), 1);

    fs::remove_file(&doc).expect("remove");
    let stale = session
        .execute_skill(&registry, "rag_stale", json!({}))
        .await
        .expect("stale");
    assert!(stale.ok);
    assert_eq!(
        stale
            .output
            .get("stale_files")
            .and_then(serde_json::Value::as_array)
            .expect("stale files")
            .len(),
        1
    );

    let deleted = session
        .execute_skill(&registry, "rag_delete_path", json!({"path": "doc.md"}))
        .await
        .expect("delete path");
    assert!(deleted.ok);
    assert_eq!(deleted.output["chunks_deleted"], json!(1));
    assert!(
        rag.search(RagQuery {
            query: "cleanup".into(),
            top_k: 5,
            scope: Some("cleanup".into()),
        })
        .expect("search after delete")
        .is_empty()
    );
}

#[tokio::test]
async fn rag_ingest_reads_workspace_files_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("doc.md"), "env routed index entry").expect("doc");
    let rag = LocalRagStore::new(temp.path().join("rag"), "jsonl").expect("rag");
    let registry = builtin_registry(SkillEnvironment {
        rag_index: rag.clone(),
        ..test_env(temp.path(), &workspace)
    });
    let reads = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(TrackingEnv {
            reads: reads.clone(),
            writes: Arc::new(AtomicUsize::new(0)),
        }),
    );

    let ingest = session
        .execute_skill(
            &registry,
            "rag_ingest",
            json!({"path": "doc.md", "scope": "env"}),
        )
        .await
        .expect("ingest");
    let search = session
        .execute_skill(
            &registry,
            "rag_search",
            json!({"query": "routed", "scope": "env"}),
        )
        .await
        .expect("search");

    assert!(ingest.ok);
    assert!(search.ok);
    assert_eq!(search.output.as_array().expect("hits").len(), 1);
    assert!(
        reads.load(Ordering::SeqCst) > 0,
        "rag ingest workspace reads must go through ExecutionEnv"
    );
}

#[tokio::test]
async fn rag_stale_checks_workspace_metadata_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let doc = workspace.join("doc.md");
    fs::write(&doc, "env stale candidate").expect("doc");
    let rag = LocalRagStore::new(temp.path().join("rag"), "jsonl").expect("rag");
    let registry = builtin_registry(SkillEnvironment {
        rag_index: rag.clone(),
        ..test_env(temp.path(), &workspace)
    });
    let reads = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(TrackingEnv {
            reads: reads.clone(),
            writes: Arc::new(AtomicUsize::new(0)),
        }),
    );
    session
        .execute_skill(
            &registry,
            "rag_ingest",
            json!({"path": "doc.md", "scope": "env"}),
        )
        .await
        .expect("ingest");
    let reads_before_stale = reads.load(Ordering::SeqCst);
    fs::remove_file(&doc).expect("remove");

    let stale = session
        .execute_skill(&registry, "rag_stale", json!({}))
        .await
        .expect("stale");

    assert!(stale.ok);
    assert_eq!(
        stale
            .output
            .get("stale_files")
            .and_then(serde_json::Value::as_array)
            .expect("stale files")
            .len(),
        1
    );
    assert!(
        reads.load(Ordering::SeqCst) > reads_before_stale,
        "rag stale metadata checks must go through ExecutionEnv"
    );
}

#[tokio::test]
async fn cloud_rag_search_requires_network_approval_before_provider_execution() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(SkillEnvironment {
        rag_config: ikaros_core::RagConfig {
            embedding_provider: "openai-compatible".into(),
            embedding_model: "embedding-model".into(),
            embedding_timeout_ms: 1000,
            embedding_max_retries: 0,
            ..ikaros_core::RagConfig::default()
        },
        rag_provider: ikaros_core::RemoteProviderConfig {
            api_key: "test-rag-key".into(),
            base_url: "https://example.invalid/v1".into(),
        },
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let result = session
        .execute_skill(&registry, "rag_search", json!({"query": "cloud retrieval"}))
        .await
        .expect("approval request");
    assert!(!result.ok);
    assert_eq!(result.output["decision"], json!("ask_user"));
    assert!(result.summary.contains("network action"));
}

#[tokio::test]
async fn cloud_rag_ingest_approval_explains_provider_and_index_write_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("doc.md"), "provider-backed ingest").expect("doc");
    let registry = builtin_registry(SkillEnvironment {
        rag_config: ikaros_core::RagConfig {
            embedding_provider: "openai-compatible".into(),
            embedding_model: "embedding-model".into(),
            embedding_timeout_ms: 1000,
            embedding_max_retries: 0,
            ..ikaros_core::RagConfig::default()
        },
        rag_provider: ikaros_core::RemoteProviderConfig {
            api_key: "test-rag-key".into(),
            base_url: "https://example.invalid/v1".into(),
        },
        ..test_env(temp.path(), &workspace)
    });
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let requested = session
        .execute_skill(
            &registry,
            "rag_ingest",
            json!({"path": "doc.md", "scope": "docs"}),
        )
        .await
        .expect("approval request");

    assert!(!requested.ok);
    assert_eq!(requested.output["decision"], json!("ask_user"));
    assert_eq!(
        requested.output["approval_context"]["operations"]["provider_call"],
        json!(true)
    );
    assert_eq!(
        requested.output["approval_context"]["operations"]["local_file_read"],
        json!(true)
    );
    assert_eq!(
        requested.output["approval_context"]["operations"]["rag_index_write"],
        json!(true)
    );
    assert_eq!(
        requested.output["approval_context"]["provider"]["embedding_provider"],
        json!("openai-compatible")
    );
    assert_eq!(
        requested.output["approval_context"]["provider"]["embedding_model"],
        json!("embedding-model")
    );
    assert_eq!(
        requested.output["approval_context"]["scope"]["path"],
        json!(workspace.join("doc.md"))
    );
    assert_eq!(
        requested.output["approval_context"]["scope"]["rag_scope"],
        json!("docs")
    );
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id");
    let approval = session
        .approvals
        .get(approval_id)
        .expect("approval lookup")
        .expect("approval record");
    assert_eq!(
        approval.request.context.as_ref().expect("context")["operations"]["rag_index_write"],
        json!(true)
    );
}

#[tokio::test]
async fn approved_cloud_rag_search_routes_embedding_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(SkillEnvironment {
        rag_config: ikaros_core::RagConfig {
            embedding_provider: "openai-compatible".into(),
            embedding_model: "embedding-model".into(),
            embedding_timeout_ms: 1000,
            embedding_max_retries: 0,
            ..ikaros_core::RagConfig::default()
        },
        rag_provider: ikaros_core::RemoteProviderConfig {
            api_key: "test-rag-key".into(),
            base_url: "https://embedding.example/v1".into(),
        },
        ..test_env(temp.path(), &workspace)
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let reads = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(ScriptedNetworkEnv {
            reads,
            calls: calls.clone(),
            response: NetworkEgressResponse {
                status: 200,
                headers: Default::default(),
                body: json!({"data": [{"embedding": [0.1, 0.2, 0.3]}]}).to_string(),
                body_bytes: None,
            },
        }),
    );

    let requested = session
        .execute_skill(&registry, "rag_search", json!({"query": "cloud retrieval"}))
        .await
        .expect("approval request");
    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id")
        .to_owned();
    session
        .decide_approval(&approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let result = session
        .execute_approved_skill(&registry, &approval_id)
        .await
        .expect("approved search");

    assert!(result.ok);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn approved_cloud_rag_ingest_routes_files_and_embedding_through_execution_env() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(workspace.join("doc.md"), "provider-backed ingest").expect("doc");
    let registry = builtin_registry(SkillEnvironment {
        rag_config: ikaros_core::RagConfig {
            embedding_provider: "openai-compatible".into(),
            embedding_model: "embedding-model".into(),
            embedding_timeout_ms: 1000,
            embedding_max_retries: 0,
            ..ikaros_core::RagConfig::default()
        },
        rag_provider: ikaros_core::RemoteProviderConfig {
            api_key: "test-rag-key".into(),
            base_url: "https://embedding.example/v1".into(),
        },
        ..test_env(temp.path(), &workspace)
    });
    let reads = Arc::new(AtomicUsize::new(0));
    let calls = Arc::new(AtomicUsize::new(0));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(
        Arc::new(ScriptedNetworkEnv {
            reads: reads.clone(),
            calls: calls.clone(),
            response: NetworkEgressResponse {
                status: 200,
                headers: Default::default(),
                body: json!({"data": [{"embedding": [0.1, 0.2, 0.3]}]}).to_string(),
                body_bytes: None,
            },
        }),
    );

    let requested = session
        .execute_skill(
            &registry,
            "rag_ingest",
            json!({"path": "doc.md", "scope": "docs"}),
        )
        .await
        .expect("approval request");
    assert!(!requested.ok);
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let approval_id = requested.output["approval_id"]
        .as_str()
        .expect("approval id")
        .to_owned();
    session
        .decide_approval(&approval_id, ApprovalStatus::Approved, None)
        .expect("approve");
    let result = session
        .execute_approved_skill(&registry, &approval_id)
        .await
        .expect("approved ingest");

    assert!(result.ok);
    assert_eq!(result.output["files_indexed"], json!(1));
    assert_eq!(result.output["chunks_indexed"], json!(1));
    assert!(
        reads.load(Ordering::SeqCst) > 0,
        "RAG ingest must read workspace files through ExecutionEnv"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "remote embedding must go through NetworkEgress"
    );
}
