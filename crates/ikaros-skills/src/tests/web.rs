// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[tokio::test]
async fn web_extract_routes_through_network_egress_and_redacts_html() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let calls = Arc::new(AtomicUsize::new(0));
    let request = Arc::new(Mutex::new(None));
    let network = Arc::new(RecordingNetwork {
        calls: calls.clone(),
        request: request.clone(),
        response: NetworkEgressResponse {
            status: 200,
            headers: [("content-type".into(), "text/html; charset=utf-8".into())].into(),
            body: "<html><head><title>Doc sk-test-secret</title><script>hidden()</script></head><body><h1>Hello</h1><p>token sk-test-secret</p></body></html>".into(),
            body_bytes: None,
        },
    });
    let env = Arc::new(NetworkedExecutionEnv::new(
        Arc::new(LocalExecutionEnv),
        network,
    ));
    let session =
        ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(env);

    let requested = session
        .execute_skill(
            &registry,
            "web_extract",
            json!({"url": "https://example.com/page", "max_chars": 256}),
        )
        .await
        .expect("approval request");
    assert!(!requested.ok);
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
        .expect("approved extract");

    assert!(result.ok);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let request = request
        .lock()
        .expect("request lock")
        .clone()
        .expect("request");
    assert_eq!(request.method, "GET");
    assert_eq!(request.url, "https://example.com/page");
    assert!(request.headers.contains_key("accept"));
    let text = result.output["text"].as_str().expect("text");
    assert!(text.contains("Hello"));
    assert!(!text.contains("sk-test-secret"));
    let output_json = serde_json::to_string(&result.output).expect("output json");
    assert!(!output_json.contains("sk-test-secret"));
    assert_eq!(
        result.output["citation"]["url"].as_str().expect("citation"),
        "https://example.com/page"
    );
}

#[tokio::test]
async fn web_extract_skips_unsupported_content_type() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let network = Arc::new(RecordingNetwork {
        calls: Arc::new(AtomicUsize::new(0)),
        request: Arc::new(Mutex::new(None)),
        response: NetworkEgressResponse {
            status: 200,
            headers: [("content-type".into(), "application/octet-stream".into())].into(),
            body: "binary-ish sk-test-secret".into(),
            body_bytes: None,
        },
    });
    let env = Arc::new(NetworkedExecutionEnv::new(
        Arc::new(LocalExecutionEnv),
        network,
    ));
    let session =
        ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(env);

    let requested = session
        .execute_skill(
            &registry,
            "web_extract",
            json!({"url": "https://example.com/blob"}),
        )
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
        .expect("approved extract");

    assert!(result.ok);
    assert_eq!(result.output["skipped"].as_bool(), Some(true));
    assert_eq!(
        result.output["reason"].as_str(),
        Some("unsupported_content_type")
    );
    assert_eq!(result.output["text"].as_str(), Some(""));
    let output_json = serde_json::to_string(&result.output).expect("output json");
    assert!(!output_json.contains("sk-test-secret"));
}

#[tokio::test]
async fn web_search_queries_provider_through_network_egress_and_returns_citations() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let calls = Arc::new(AtomicUsize::new(0));
    let request = Arc::new(Mutex::new(None));
    let network = Arc::new(RecordingNetwork {
        calls: calls.clone(),
        request: request.clone(),
        response: NetworkEgressResponse {
            status: 200,
            headers: [("content-type".into(), "text/html; charset=utf-8".into())].into(),
            body: r#"
                <html><body>
                  <a rel="nofollow" class="result__a" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fexample.com%2Fdocs">Example Docs sk-search-secret</a>
                  <a class="result__snippet">Useful docs snippet sk-search-secret</a>
                  <a rel="nofollow" class="result__a" href="https://example.org/guide">Guide</a>
                  <a class="result__snippet">Second snippet</a>
                </body></html>
            "#
            .into(),
            body_bytes: None,
        },
    });
    let env = Arc::new(NetworkedExecutionEnv::new(
        Arc::new(LocalExecutionEnv),
        network,
    ));
    let session =
        ExecutionSession::new(&workspace, temp.path().join("audit")).with_execution_env(env);

    let requested = session
        .execute_skill(
            &registry,
            "web_search",
            json!({
                "query": "ikaros runtime",
                "endpoint": "https://duckduckgo.com/html/",
                "max_results": 2
            }),
        )
        .await
        .expect("approval request");
    assert!(!requested.ok);
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
    let request = request
        .lock()
        .expect("request lock")
        .clone()
        .expect("request");
    assert_eq!(request.method, "GET");
    assert!(request.url.starts_with("https://duckduckgo.com/html/"));
    assert!(request.url.contains("q=ikaros+runtime"));
    assert_eq!(result.output["result_count"].as_u64(), Some(2));
    assert_eq!(
        result.output["results"][0]["url"].as_str(),
        Some("https://example.com/docs")
    );
    assert_eq!(
        result.output["results"][0]["citation"]["url"].as_str(),
        Some("https://example.com/docs")
    );
    let output_json = serde_json::to_string(&result.output).expect("output json");
    assert!(!output_json.contains("sk-search-secret"));
}

#[tokio::test]
async fn web_extract_rejects_non_http_urls_before_network_egress() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));
    let error = WebExtractSkill
        .execute(
            json!({"url": "file:///etc/passwd"}),
            session.skill_context(),
        )
        .await
        .expect_err("unsupported scheme");

    assert!(error.to_string().contains("unsupported"));
}

#[test]
fn persona_loader_skill_uses_default_parser_type() {
    let persona = PersonaLoader::parse(PersonaLoader::default_markdown()).expect("persona");
    assert_eq!(persona.identity.name, "Ikaros");
}
