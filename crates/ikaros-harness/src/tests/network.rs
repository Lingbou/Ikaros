// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn skill_registry_reports_unified_tool_visibility() {
    let mut registry = SkillRegistry::new();
    registry.register_with_toolset(CoreReadSkill, Toolset::Core);
    registry.register_with_toolset(RagDeferredSkill, Toolset::Rag);
    registry.register_with_toolset(HiddenExecutableSkill, Toolset::Plugin);
    registry.register_prompt_skill(
        SkillDescriptor {
            name: "prompt_doc_test".into(),
            description: "Prompt-only test skill.".into(),
            input_schema: json!({"type": "object"}),
            risk_level: RiskLevel::SafeRead,
            kind: SkillDescriptorKind::PromptSkill,
            disable_model_invocation: true,
            execution_mode: ToolExecutionMode::Sequential,
            toolset: Toolset::Coding,
            timeout_ms: None,
            provenance: None,
            support_files: Vec::new(),
        },
        "Prompt-only instructions.",
    );

    let default = ToolsetSelection::default();
    assert_eq!(
        registry.visibility_for("core_read_test", &default),
        Some(ToolVisibility::Direct)
    );
    assert_eq!(
        registry.visibility_for("rag_deferred_test", &default),
        Some(ToolVisibility::Disabled)
    );
    assert_eq!(
        registry.visibility_for("hidden_executable_test", &default),
        Some(ToolVisibility::Hidden)
    );

    let expanded = ToolsetSelection::new([
        Toolset::Core,
        Toolset::Workspace,
        Toolset::Memory,
        Toolset::Rag,
        Toolset::Coding,
        Toolset::Plugin,
    ]);
    assert_eq!(
        registry.visibility_for("rag_deferred_test", &expanded),
        Some(ToolVisibility::Deferred)
    );
    assert_eq!(
        registry.visibility_for("prompt_doc_test", &expanded),
        Some(ToolVisibility::Deferred)
    );
    assert_eq!(registry.visibility_for("missing", &expanded), None);
}

#[test]
fn tool_registry_is_executable_tool_view_not_skill_registry_alias() {
    let mut registry = SkillRegistry::new();
    registry.register_with_toolset(CoreReadSkill, Toolset::Core);
    registry.register_with_toolset(RagDeferredSkill, Toolset::Rag);
    registry.register_prompt_skill(
        SkillDescriptor {
            name: "prompt_doc_test".into(),
            description: "Prompt-only test skill.".into(),
            input_schema: json!({"type": "object"}),
            risk_level: RiskLevel::SafeRead,
            kind: SkillDescriptorKind::PromptSkill,
            disable_model_invocation: true,
            execution_mode: ToolExecutionMode::Sequential,
            toolset: Toolset::Coding,
            timeout_ms: None,
            provenance: None,
            support_files: Vec::new(),
        },
        "Prompt-only instructions.",
    );

    let tool_registry = registry.tool_registry();
    let expanded = ToolsetSelection::new([
        Toolset::Core,
        Toolset::Workspace,
        Toolset::Memory,
        Toolset::Rag,
        Toolset::Coding,
    ]);
    let descriptors = tool_registry.descriptors_for(&expanded);
    let names = descriptors
        .iter()
        .map(|descriptor| descriptor.name.as_str())
        .collect::<Vec<_>>();

    assert!(names.contains(&"core_read_test"));
    assert!(names.contains(&"rag_deferred_test"));
    assert!(!names.contains(&"prompt_doc_test"));
    assert!(
        descriptors
            .iter()
            .all(|descriptor| descriptor.kind == SkillDescriptorKind::ExecutableTool)
    );
    assert_eq!(
        tool_registry.visibility_for("prompt_doc_test", &expanded),
        None
    );
    assert!(tool_registry.get("core_read_test").is_some());
    assert!(tool_registry.get("prompt_doc_test").is_none());
}

#[test]
#[should_panic(expected = "unsupported toolset")]
fn execution_session_rejects_invalid_agent_toolset_names() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut profile = AgentProfile::build();
    profile.toolsets = vec!["core".into(), "made-up-toolset".into()];
    let agent = ResolvedAgentProfile {
        name: "invalid-toolset-agent".into(),
        profile,
    };

    let _session = ExecutionSession::new_with_agent(
        temp.path().join("workspace"),
        temp.path().join("audit"),
        &agent,
    );
}

#[tokio::test]
async fn network_egress_denies_by_default() {
    let env = GovernedNetworkEgress::deny_by_default();
    let result = env
        .send_network_request(NetworkEgressRequest {
            method: "GET".into(),
            url: "https://api.example/v1/models".into(),
            headers: BTreeMap::new(),
            body: None,
            body_bytes: None,
        })
        .await;

    assert!(result.is_err());
    assert!(
        result
            .expect_err("denied")
            .to_string()
            .contains("network egress denied")
    );
}

#[tokio::test]
async fn network_egress_allowlist_matches_host_without_leaking_secret_url() {
    let policy = NetworkEgressPolicy::allow_hosts(["api.example".into()]);

    assert!(
        policy
            .allows("https://api.example/v1/chat?key=sk-secret")
            .is_ok()
    );

    let denied = policy
        .allows("https://evil.example/v1/chat?key=sk-secret")
        .expect_err("denied");
    let message = denied.to_string();
    assert!(message.contains("network egress denied"));
    assert!(message.contains("evil.example"));
    assert!(!message.contains("sk-secret"));
}

#[tokio::test]
async fn network_egress_policy_denies_private_ip_literals_but_allows_explicit_loopback() {
    let private_policy = NetworkEgressPolicy::allow_hosts(["192.168.1.10".into()]);
    let denied = private_policy
        .allows("http://192.168.1.10/v1/models")
        .expect_err("private literal denied");
    assert!(denied.to_string().contains("restricted IP address"));

    let loopback_policy = NetworkEgressPolicy::allow_hosts(["127.0.0.1".into()]);
    loopback_policy
        .allows("http://127.0.0.1:11434/api/chat")
        .expect("explicit loopback remains available for local providers");
}

#[tokio::test]
async fn network_egress_does_not_allow_suffix_host_confusion() {
    let policy = NetworkEgressPolicy::allow_hosts(["api.example".into()]);

    let denied = policy
        .allows("https://not-api.example/v1/chat")
        .expect_err("suffix host denied");

    assert!(denied.to_string().contains("network egress denied"));
}

#[test]
fn network_egress_request_debug_redacts_secret_headers_url_and_body() {
    let mut headers = BTreeMap::new();
    headers.insert(
        "authorization".into(),
        "Bearer sk-testsecret1234567890abcdef".into(),
    );
    headers.insert("x-api-key".into(), "sk-headersecret1234567890abcdef".into());
    headers.insert("content-type".into(), "application/json".into());
    let request = NetworkEgressRequest {
        method: "POST".into(),
        url: "https://api.example/v1/embeddings?api_key=sk-urlsecret1234567890abcdef".into(),
        headers,
        body: Some(
            r#"{"input":"hello","api_key":"plain-body-key","nested":{"access_token":"plain-body-token"}}"#
                .into(),
        ),
        body_bytes: None,
    };

    let debug = format!("{request:?}");

    assert!(debug.contains("NetworkEgressRequest"));
    assert!(debug.contains("[REDACTED_SECRET]"));
    assert!(!debug.contains("sk-testsecret1234567890abcdef"));
    assert!(!debug.contains("sk-headersecret1234567890abcdef"));
    assert!(!debug.contains("sk-urlsecret1234567890abcdef"));
    assert!(!debug.contains("plain-body-key"));
    assert!(!debug.contains("plain-body-token"));
}

#[test]
fn network_egress_response_debug_redacts_secret_body() {
    let response = NetworkEgressResponse {
        status: 401,
        headers: BTreeMap::from([
            ("content-type".into(), "application/json".into()),
            ("set-cookie".into(), "session=plain-response-cookie".into()),
        ]),
        body: r#"{"error":"bad key","api_key":"plain-response-key","details":{"token":"plain-response-token"}}"#
            .into(),
        body_bytes: None,
    };

    let debug = format!("{response:?}");

    assert!(debug.contains("NetworkEgressResponse"));
    assert!(debug.contains("[REDACTED_SECRET]"));
    assert!(!debug.contains("plain-response-key"));
    assert!(!debug.contains("plain-response-token"));
    assert!(!debug.contains("plain-response-cookie"));
}

#[tokio::test]
async fn workspace_env_denies_reading_symlink_escape() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("workspace");
    let secret = temp.path().join("outside-secret.txt");
    fs::write(&secret, "do not read").expect("secret");
    let link = workspace.join("secret-link.txt");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&secret, &link).expect("symlink");
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(&secret, &link).expect("symlink");

    let env = WorkspaceExecutionEnv::local(&workspace);
    let error = env
        .read_to_string(Path::new("secret-link.txt"))
        .await
        .expect_err("symlink escape denied");

    assert!(matches!(error, ikaros_core::IkarosError::OutOfScope(_)));
}

#[tokio::test]
async fn workspace_env_allows_explicit_plugin_cwd_only_for_plugin_programs() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let plugin_dir = temp.path().join("skills/hello");
    fs::create_dir_all(&workspace).expect("workspace");
    fs::create_dir_all(&plugin_dir).expect("plugin");
    let runner = plugin_dir.join(if cfg!(windows) {
        "runner.cmd"
    } else {
        "runner.sh"
    });
    fs::write(
        &runner,
        if cfg!(windows) {
            "@echo off\r\nfor %%I in (.) do echo %%~nxI\r\n"
        } else {
            "#!/bin/sh\nbasename \"$PWD\"\n"
        },
    )
    .expect("runner");
    #[cfg(unix)]
    fs::set_permissions(&runner, {
        let mut permissions = fs::metadata(&runner).expect("metadata").permissions();
        permissions.set_mode(0o755);
        permissions
    })
    .expect("chmod");
    let env = WorkspaceExecutionEnv::local(&workspace);

    let denied = env
        .run_process(ProcessRequest::program(
            runner.display().to_string(),
            vec![],
            &plugin_dir,
        ))
        .await
        .expect_err("default workspace process scope denies plugin cwd");
    assert!(matches!(denied, ikaros_core::IkarosError::OutOfScope(_)));

    let allowed = env
        .run_process(
            ProcessRequest::program(runner.display().to_string(), vec![], &plugin_dir)
                .with_plugin_cwd_scope(),
        )
        .await
        .expect("plugin cwd allowed");
    assert_eq!(allowed.stdout.trim(), "hello");

    let outside_program = workspace.join(if cfg!(windows) { "tool.cmd" } else { "tool.sh" });
    fs::write(
        &outside_program,
        if cfg!(windows) {
            "@echo off\r\necho outside\r\n"
        } else {
            "#!/bin/sh\necho outside\n"
        },
    )
    .expect("outside");
    #[cfg(unix)]
    fs::set_permissions(&outside_program, {
        let mut permissions = fs::metadata(&outside_program)
            .expect("metadata")
            .permissions();
        permissions.set_mode(0o755);
        permissions
    })
    .expect("chmod");

    let rejected = env
        .run_process(
            ProcessRequest::program(outside_program.display().to_string(), vec![], &plugin_dir)
                .with_plugin_cwd_scope(),
        )
        .await
        .expect_err("plugin cwd scope rejects program outside plugin cwd");
    assert!(matches!(rejected, ikaros_core::IkarosError::OutOfScope(_)));
}

#[tokio::test]
async fn networked_execution_env_routes_network_to_governed_egress() {
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = Arc::new(CountingNetworkTransport {
        calls: calls.clone(),
    });
    let policy = NetworkEgressPolicy::allow_hosts(["api.example".into()]);
    let egress = Arc::new(GovernedNetworkEgress::new(policy, transport));
    let env = NetworkedExecutionEnv::new(Arc::new(LocalExecutionEnv), egress);

    let mut headers = BTreeMap::new();
    headers.insert("authorization".into(), "Bearer sk-test".into());
    let response = env
        .send_network_request(NetworkEgressRequest {
            method: "POST".into(),
            url: "https://api.example/v1/chat".into(),
            headers,
            body: Some("{\"ok\":true}".into()),
            body_bytes: None,
        })
        .await
        .expect("allowed request");

    assert_eq!(response.status, 200);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn network_egress_policy_denies_non_http_url_schemes() {
    let calls = Arc::new(AtomicUsize::new(0));
    let transport = Arc::new(CountingNetworkTransport {
        calls: calls.clone(),
    });
    let policy = NetworkEgressPolicy::allow_hosts(["api.example".into()]);
    let egress = Arc::new(GovernedNetworkEgress::new(policy, transport));
    let env = NetworkedExecutionEnv::new(Arc::new(LocalExecutionEnv), egress);

    let error = env
        .send_network_request(NetworkEgressRequest {
            method: "GET".into(),
            url: "ftp://api.example/archive".into(),
            headers: Default::default(),
            body: None,
            body_bytes: None,
        })
        .await
        .expect_err("non-http scheme should be denied before transport");

    assert!(error.to_string().contains("unsupported URL scheme"));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
