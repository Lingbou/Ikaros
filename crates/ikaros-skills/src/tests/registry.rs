// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn builtin_registry_contains_core_skill_groups() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let names = registry.names();

    for expected in [
        "fs_read",
        "memory_append",
        "rag_ingest",
        "voice_tts",
        "repo_scan",
        "code_edit_guarded",
        "task_summarize",
    ] {
        assert!(names.contains(&expected.to_string()), "missing {expected}");
    }
}

#[test]
fn builtin_registry_filters_model_visible_tools_by_toolset() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));

    let default_visible = registry.model_visible_names_for(&ToolsetSelection::default());
    for expected in ["fs_read", "list_dir", "git_status", "memory_search"] {
        assert!(
            default_visible.contains(&expected.to_string()),
            "default visible tools should include {expected}: {default_visible:?}"
        );
    }
    for bridge in ["tool_search", "tool_describe", "tool_call"] {
        assert!(
            default_visible.contains(&bridge.to_string()),
            "default visible tools should include bridge tool {bridge}: {default_visible:?}"
        );
    }
    for deferred in [
        "rag_search",
        "rag_ingest",
        "voice_tts",
        "voice_asr",
        "code_workflow",
        "code_edit_guarded",
        "plugin_command_run",
    ] {
        assert!(
            !default_visible.contains(&deferred.to_string()),
            "default visible tools should defer {deferred}: {default_visible:?}"
        );
    }

    let coding_visible = registry.model_visible_names_for(&ToolsetSelection::new([
        Toolset::Core,
        Toolset::Workspace,
        Toolset::Memory,
        Toolset::Coding,
    ]));
    assert!(!coding_visible.contains(&"code_workflow".to_string()));
    assert!(!coding_visible.contains(&"code_edit_guarded".to_string()));
    assert!(coding_visible.contains(&"tool_search".to_string()));
    assert!(coding_visible.contains(&"tool_describe".to_string()));
    assert!(coding_visible.contains(&"tool_call".to_string()));
    assert!(!coding_visible.contains(&"rag_search".to_string()));

    let rag_visible = registry.model_visible_names_for(&ToolsetSelection::new([
        Toolset::Core,
        Toolset::Workspace,
        Toolset::Memory,
        Toolset::Rag,
    ]));
    assert!(!rag_visible.contains(&"rag_search".to_string()));
    assert!(rag_visible.contains(&"tool_search".to_string()));
    assert!(rag_visible.contains(&"tool_describe".to_string()));
    assert!(rag_visible.contains(&"tool_call".to_string()));
    assert!(!rag_visible.contains(&"voice_tts".to_string()));
}

#[tokio::test]
async fn tool_search_discovers_deferred_tools_without_direct_exposure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let mut profile = AgentProfile::build();
    profile.toolsets = vec![
        "core".into(),
        "workspace".into(),
        "memory".into(),
        "rag".into(),
    ];
    let agent = ResolvedAgentProfile {
        name: "rag-tools".into(),
        profile,
    };
    let session = ExecutionSession::new_with_agent(&workspace, temp.path().join("audit"), &agent);

    let result = session
        .execute_skill(&registry, "tool_search", json!({"query": "rag"}))
        .await
        .expect("tool_search");

    assert!(result.ok, "{result:?}");
    let tools = result.output["tools"].as_array().expect("tools array");
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == "rag_search" && tool["toolset"] == "rag"),
        "rag_search should be discoverable: {tools:?}"
    );
    assert!(
        !tools.iter().any(|tool| tool["name"] == "fs_read"),
        "direct-visible tools should not be returned by deferred search: {tools:?}"
    );
}

#[tokio::test]
async fn default_build_agent_discovers_deferred_toolsets_without_direct_exposure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let agent = ResolvedAgentProfile {
        name: "default-build".into(),
        profile: AgentProfile::build(),
    };
    let session = ExecutionSession::new_with_agent(&workspace, temp.path().join("audit"), &agent);
    let selection = ToolsetSelection::from_names(agent.profile.toolsets.iter()).expect("toolsets");
    let visible = registry.model_visible_names_for(&selection);

    for deferred in ["rag_search", "code_workflow", "voice_tts"] {
        assert!(
            !visible.contains(&deferred.to_string()),
            "default build profile must not directly expose deferred tool {deferred}: {visible:?}"
        );
    }

    let result = session
        .execute_skill(&registry, "tool_search", json!({"query": "workflow"}))
        .await
        .expect("tool_search");
    let tools = result.output["tools"].as_array().expect("tools array");
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == "code_workflow" && tool["toolset"] == "coding"),
        "default build profile should discover coding tools through bridge: {tools:?}"
    );

    let rag = session
        .execute_skill(&registry, "tool_search", json!({"query": "rag"}))
        .await
        .expect("tool_search rag");
    let rag_tools = rag.output["tools"].as_array().expect("rag tools array");
    assert!(
        rag_tools
            .iter()
            .any(|tool| tool["name"] == "rag_search" && tool["toolset"] == "rag"),
        "default build profile should discover RAG tools through bridge: {rag_tools:?}"
    );
}

#[tokio::test]
async fn tool_search_rejects_blank_query_without_disclosing_deferred_tools() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let mut profile = AgentProfile::build();
    profile.toolsets = vec![
        "core".into(),
        "workspace".into(),
        "memory".into(),
        "rag".into(),
    ];
    let agent = ResolvedAgentProfile {
        name: "rag-tools".into(),
        profile,
    };
    let session = ExecutionSession::new_with_agent(&workspace, temp.path().join("audit"), &agent);

    let error = session
        .execute_skill(&registry, "tool_search", json!({"query": "   "}))
        .await
        .expect_err("blank tool_search query should be rejected");

    assert!(error.to_string().contains("query"));
    let call_error = session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "rag_search", "input": {"query": "anything"}}),
        )
        .await
        .expect_err("blank search must not disclose deferred tools");
    assert!(call_error.to_string().contains("has not been disclosed"));
}

#[tokio::test]
async fn tool_call_routes_deferred_skill_through_harness_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(workspace.join(".temp")).expect("mkdir");
    fs::write(workspace.join(".temp/secret.md"), "secret").expect("write");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let mut profile = AgentProfile::build();
    profile.toolsets = vec![
        "core".into(),
        "workspace".into(),
        "memory".into(),
        "rag".into(),
    ];
    let agent = ResolvedAgentProfile {
        name: "rag-tools".into(),
        profile,
    };
    let session = ExecutionSession::new_with_agent(&workspace, temp.path().join("audit"), &agent);

    session
        .execute_skill(&registry, "tool_describe", json!({"name": "rag_ingest"}))
        .await
        .expect("tool_describe");
    let result = session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "rag_ingest", "input": {"path": ".temp/secret.md"}}),
        )
        .await
        .expect("tool_call");

    assert!(result.ok, "{result:?}");
    assert_eq!(result.output["result"]["ok"], json!(false));
    assert!(
        result.summary.contains(".temp"),
        "deferred call should surface inner policy result: {}",
        result.summary
    );
    let audit_events = session.audit.read_all().expect("audit");
    let audited_tool_names = audit_events
        .iter()
        .filter(|event| event.kind == "tool_call")
        .map(|event| event.data["name"].as_str().unwrap_or_default().to_owned())
        .collect::<Vec<_>>();
    assert!(
        audited_tool_names.contains(&"tool_call".to_string()),
        "bridge call should remain audited: {audited_tool_names:?}"
    );
    assert!(
        audited_tool_names.contains(&"rag_ingest".to_string()),
        "underlying deferred tool should be audited: {audited_tool_names:?}"
    );
    let deferred_invocation = audit_events
        .iter()
        .find(|event| event.kind == "deferred_tool_invocation")
        .expect("deferred invocation linkage event");
    assert_eq!(deferred_invocation.data["bridge_tool"], json!("tool_call"));
    assert_eq!(deferred_invocation.data["target_tool"], json!("rag_ingest"));
    assert_eq!(deferred_invocation.data["target_toolset"], json!("rag"));
    assert_eq!(
        deferred_invocation.data["target_kind"],
        json!("executable_tool")
    );
    assert_eq!(deferred_invocation.data["target_callable"], json!(true));
}

#[tokio::test]
async fn tool_call_requires_prior_deferred_tool_disclosure() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let mut profile = AgentProfile::build();
    profile.toolsets = vec![
        "core".into(),
        "workspace".into(),
        "memory".into(),
        "rag".into(),
    ];
    let agent = ResolvedAgentProfile {
        name: "rag-tools".into(),
        profile,
    };
    let session = ExecutionSession::new_with_agent(&workspace, temp.path().join("audit"), &agent);

    let error = session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "rag_stale", "input": {}}),
        )
        .await
        .expect_err("tool_call should require search or describe disclosure");

    assert!(error.to_string().contains("has not been disclosed"));
    assert!(error.to_string().contains("tool_search"));
    assert!(error.to_string().contains("tool_describe"));

    session
        .execute_skill(&registry, "tool_describe", json!({"name": "rag_stale"}))
        .await
        .expect("tool_describe");
    let result = session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "rag_stale", "input": {}}),
        )
        .await
        .expect("tool_call after describe");

    assert!(result.ok, "{result:?}");
}

#[tokio::test]
async fn deferred_tool_disclosure_is_scoped_to_execution_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let mut profile = AgentProfile::build();
    profile.toolsets = vec![
        "core".into(),
        "workspace".into(),
        "memory".into(),
        "rag".into(),
    ];
    let agent = ResolvedAgentProfile {
        name: "rag-tools".into(),
        profile,
    };
    let first_session =
        ExecutionSession::new_with_agent(&workspace, temp.path().join("audit-a"), &agent);
    let second_session =
        ExecutionSession::new_with_agent(&workspace, temp.path().join("audit-b"), &agent);

    first_session
        .execute_skill(&registry, "tool_search", json!({"query": "rag_stale"}))
        .await
        .expect("tool_search");
    let first_result = first_session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "rag_stale", "input": {}}),
        )
        .await
        .expect("tool_call in first session");
    assert!(first_result.ok, "{first_result:?}");

    let error = second_session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "rag_stale", "input": {}}),
        )
        .await
        .expect_err("disclosure must not leak across sessions");

    assert!(error.to_string().contains("has not been disclosed"));
}

#[tokio::test]
async fn tool_call_rejects_deferred_tools_outside_agent_toolset() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let mut profile = AgentProfile::build();
    profile.toolsets = vec!["core".into(), "workspace".into(), "memory".into()];
    let agent = ResolvedAgentProfile {
        name: "restricted-toolset".into(),
        profile,
    };
    let session = ExecutionSession::new_with_agent(&workspace, temp.path().join("audit"), &agent);

    let error = session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "rag_search", "input": {"query": "anything"}}),
        )
        .await
        .expect_err("rag_search should require the rag toolset");

    assert!(error.to_string().contains("toolset"));
    assert!(error.to_string().contains("rag"));
}

#[tokio::test]
async fn tool_search_hides_deferred_tools_outside_agent_toolset() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let mut profile = AgentProfile::build();
    profile.toolsets = vec!["core".into(), "workspace".into(), "memory".into()];
    let agent = ResolvedAgentProfile {
        name: "restricted-toolset".into(),
        profile,
    };
    let session = ExecutionSession::new_with_agent(&workspace, temp.path().join("audit"), &agent);

    let result = session
        .execute_skill(&registry, "tool_search", json!({"query": "rag"}))
        .await
        .expect("tool_search");

    assert!(result.ok, "{result:?}");
    let tools = result.output["tools"].as_array().expect("tools array");
    assert!(
        !tools.iter().any(|tool| tool["name"] == "rag_search"),
        "restricted agent should not discover rag tools: {tools:?}"
    );
}

#[tokio::test]
async fn explicit_only_executable_tools_are_not_bridge_callable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let mut registry = SkillRegistry::new();
    registry.register_with_toolset(HiddenExplicitOnlySkill, Toolset::Coding);
    let deferred_registry = registry.clone();
    registry.register_with_toolset(
        ToolSearchSkill::new(deferred_registry.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(
        ToolDescribeSkill::new(deferred_registry.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(ToolCallSkill::new(deferred_registry), Toolset::Core);
    let mut profile = AgentProfile::build();
    profile.toolsets = vec![
        "core".into(),
        "workspace".into(),
        "memory".into(),
        "coding".into(),
    ];
    let agent = ResolvedAgentProfile {
        name: "coding-tools".into(),
        profile,
    };
    let session = ExecutionSession::new_with_agent(&workspace, temp.path().join("audit"), &agent);

    assert!(
        !registry
            .model_visible_names_for(&ToolsetSelection::new([Toolset::Core, Toolset::Coding]))
            .contains(&"hidden_explicit_only".to_string()),
        "explicit-only executable tools must not enter provider tool schema"
    );

    let search = session
        .execute_skill(&registry, "tool_search", json!({"query": "hidden"}))
        .await
        .expect("tool_search");
    let tools = search.output["tools"].as_array().expect("tools array");
    assert!(
        !tools
            .iter()
            .any(|tool| tool["name"] == "hidden_explicit_only"),
        "explicit-only executable tools must not be discoverable through tool_search: {tools:?}"
    );

    let describe_error = session
        .execute_skill(
            &registry,
            "tool_describe",
            json!({"name": "hidden_explicit_only"}),
        )
        .await
        .expect_err("tool_describe must not expose explicit-only executable tools");
    assert!(describe_error.to_string().contains("not found"));

    session.disclose_deferred_tool("hidden_explicit_only");
    let call_error = session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "hidden_explicit_only", "input": {}}),
        )
        .await
        .expect_err("tool_call must not execute explicit-only tools even if name is known");
    assert!(call_error.to_string().contains("not found"));
}

#[tokio::test]
async fn bridge_tools_use_default_toolsets_when_session_has_no_agent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let session = ExecutionSession::new(&workspace, temp.path().join("audit"));

    let search = session
        .execute_skill(&registry, "tool_search", json!({"query": "rag"}))
        .await
        .expect("tool_search");
    let tools = search.output["tools"].as_array().expect("tools array");
    assert!(
        !tools.iter().any(|tool| tool["name"] == "rag_search"),
        "unbound sessions should not discover non-default deferred toolsets: {tools:?}"
    );

    let error = session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "rag_search", "input": {"query": "anything"}}),
        )
        .await
        .expect_err("rag_search should require an agent/profile that enables rag");
    assert!(error.to_string().contains("toolset"));
    assert!(error.to_string().contains("rag"));
}

#[tokio::test]
async fn prompt_skills_are_described_but_never_executed_as_tools() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    fs::create_dir_all(&workspace).expect("mkdir");
    let mut registry = SkillRegistry::new();
    registry.register_prompt_skill(
        SkillDescriptor {
            name: "rust_review_guide".into(),
            description: "Instructions for reviewing Rust runtime changes.".into(),
            input_schema: json!({"type": "object"}),
            risk_level: ikaros_core::RiskLevel::SafeRead,
            kind: SkillDescriptorKind::PromptSkill,
            disable_model_invocation: true,
            execution_mode: ToolExecutionMode::Sequential,
            toolset: Toolset::Coding,
            timeout_ms: None,
            provenance: Some("test-fixture".into()),
            support_files: vec![],
        },
        "Review Rust changes for policy bypasses, session evidence, replay behavior, and token=abc123 handling.",
    );
    let deferred_registry = registry.clone();
    registry.register_with_toolset(
        ToolSearchSkill::new(deferred_registry.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(
        ToolDescribeSkill::new(deferred_registry.clone()),
        Toolset::Core,
    );
    registry.register_with_toolset(ToolCallSkill::new(deferred_registry), Toolset::Core);
    let mut profile = AgentProfile::build();
    profile.toolsets = vec![
        "core".into(),
        "workspace".into(),
        "memory".into(),
        "coding".into(),
    ];
    let agent = ResolvedAgentProfile {
        name: "coding-docs".into(),
        profile,
    };
    let session = ExecutionSession::new_with_agent(&workspace, temp.path().join("audit"), &agent);

    assert!(
        !registry
            .model_visible_names_for(&ToolsetSelection::new([Toolset::Core, Toolset::Coding]))
            .contains(&"rust_review_guide".to_string()),
        "prompt skills must not enter the provider tool schema"
    );

    let search = session
        .execute_skill(&registry, "tool_search", json!({"query": "rust"}))
        .await
        .expect("tool_search");
    let tools = search.output["tools"].as_array().expect("tools array");
    assert!(tools.iter().any(|tool| {
        tool["name"] == "rust_review_guide"
            && tool["kind"] == "prompt_skill"
            && tool["callable"] == false
    }));

    let describe = session
        .execute_skill(
            &registry,
            "tool_describe",
            json!({"name": "rust_review_guide"}),
        )
        .await
        .expect("tool_describe");
    let instructions = describe.output["skill"]["instructions"]
        .as_str()
        .expect("instructions");
    assert!(!instructions.contains("token=abc123"));
    assert!(instructions.contains("token=[REDACTED_SECRET]"));

    let error = session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "rust_review_guide", "input": {}}),
        )
        .await
        .expect_err("prompt skills are not executable tools");
    assert!(error.to_string().contains("prompt skill"));
    assert!(error.to_string().contains("not executable"));
}

#[tokio::test]
async fn builtin_registry_loads_prompt_skill_documents_from_skills_dir() {
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let skill_dir = temp.path().join("skills/rust_review");
    fs::create_dir_all(&skill_dir).expect("skill dir");
    fs::create_dir_all(&workspace).expect("mkdir");
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
description: Review Rust runtime changes for safety and replay evidence.
toolset: coding
provenance: local-skill-doc
support_files: [CHECKLIST.md, references/replay.md, ../escape.md]
---
# Rust Review

Check policy bypasses, session evidence, replay behavior, and token=abc123 handling.
"#,
    )
    .expect("skill doc");
    fs::write(
        skill_dir.join("CHECKLIST.md"),
        "This support file should be loaded by tool_describe with token=abc123 redacted.",
    )
    .expect("support file");
    fs::create_dir_all(skill_dir.join("references")).expect("references dir");
    fs::write(
        skill_dir.join("references/replay.md"),
        "Replay evidence guidance should be loaded only after tool_describe.",
    )
    .expect("nested support file");
    let registry = builtin_registry(test_env(temp.path(), &workspace));
    let mut profile = AgentProfile::build();
    profile.toolsets = vec![
        "core".into(),
        "workspace".into(),
        "memory".into(),
        "coding".into(),
    ];
    let agent = ResolvedAgentProfile {
        name: "coding-docs".into(),
        profile,
    };
    let session = ExecutionSession::new_with_agent(&workspace, temp.path().join("audit"), &agent);

    assert!(
        !registry
            .model_visible_names_for(&ToolsetSelection::new([Toolset::Core, Toolset::Coding]))
            .contains(&"rust_review".to_string()),
        "loaded prompt skill documents must not enter the provider tool schema"
    );

    let search = session
        .execute_skill(&registry, "tool_search", json!({"query": "runtime"}))
        .await
        .expect("tool_search");
    let tools = search.output["tools"].as_array().expect("tools array");
    assert!(tools.iter().any(|tool| {
        tool["name"] == "rust_review"
            && tool["kind"] == "prompt_skill"
            && tool["toolset"] == "coding"
            && tool["callable"] == false
            && tool["provenance"] == "local-skill-doc"
            && tool["support_file_count"] == 3
            && tool["support_files"] == json!(["SKILL.md", "CHECKLIST.md", "references/replay.md"])
            && tool.get("instructions").is_none()
    }));

    let describe = session
        .execute_skill(&registry, "tool_describe", json!({"name": "rust_review"}))
        .await
        .expect("tool_describe");
    let descriptor = &describe.output["skill"]["descriptor"];
    assert_eq!(descriptor["provenance"], json!("local-skill-doc"));
    assert_eq!(
        descriptor["support_files"],
        json!(["SKILL.md", "CHECKLIST.md", "references/replay.md"])
    );
    let instructions = describe.output["skill"]["instructions"]
        .as_str()
        .expect("instructions");
    assert!(!instructions.contains("token=abc123"));
    assert!(instructions.contains("token=[REDACTED_SECRET]"));
    let support_files = describe.output["skill"]["support_files"]
        .as_array()
        .expect("support files");
    assert_eq!(support_files.len(), 3);
    assert_eq!(support_files[0]["path"], json!("SKILL.md"));
    assert_eq!(support_files[0]["content"], instructions);
    assert_eq!(support_files[1]["path"], json!("CHECKLIST.md"));
    assert_eq!(
        support_files[1]["content"],
        json!(
            "This support file should be loaded by tool_describe with token=[REDACTED_SECRET] redacted."
        )
    );
    assert_eq!(support_files[2]["path"], json!("references/replay.md"));
    assert_eq!(
        support_files[2]["content"],
        json!("Replay evidence guidance should be loaded only after tool_describe.")
    );

    let error = session
        .execute_skill(
            &registry,
            "tool_call",
            json!({"name": "rust_review", "input": {}}),
        )
        .await
        .expect_err("prompt skill documents are not executable");
    assert!(error.to_string().contains("prompt skill"));
}
