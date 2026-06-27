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
