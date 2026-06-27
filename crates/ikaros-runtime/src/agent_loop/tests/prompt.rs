// SPDX-License-Identifier: GPL-3.0-only

use super::*;

#[test]
fn agent_loop_prompt_explains_deferred_tool_disclosure_when_bridge_tools_are_visible() {
    let prompt = build_agent_loop_system_prompt(
        "Base system",
        &[
            test_tool_definition("tool_search"),
            test_tool_definition("tool_describe"),
            test_tool_definition("tool_call"),
        ],
        &HeuristicTokenEstimator,
    )
    .prompt;

    assert!(prompt.contains("tool_search"));
    assert!(prompt.contains("tool_describe"));
    assert!(prompt.contains("tool_call"));
    assert!(prompt.contains("disclosed"));
}

#[test]
fn agent_loop_prompt_omits_deferred_tool_guidance_without_bridge_tools() {
    let report = build_agent_loop_system_prompt(
        "Base system",
        &[test_tool_definition("fs_read")],
        &HeuristicTokenEstimator,
    );

    assert!(
        !report
            .sections
            .iter()
            .any(|section| section.title == "Deferred tool disclosure"),
        "{:?}",
        report.sections
    );
    assert!(!report.prompt.contains("tool_search"));
    assert!(!report.prompt.contains("tool_describe"));
}

#[test]
fn agent_loop_prompt_builder_returns_observable_tool_guidance_sections() {
    let report = build_agent_loop_system_prompt(
        "Base system token=abc123",
        &[
            super::super::AgentLoopToolDefinition {
                name: "tool_search".into(),
                description: "Search deferred tools token=abc123".into(),
                input_schema: json!({"type": "object"}),
                risk: RiskLevel::SafeRead,
                execution_mode: ToolExecutionMode::Parallel,
                timeout_ms: Some(1000),
            },
            test_tool_definition("tool_describe"),
            test_tool_definition("tool_call"),
        ],
        &HeuristicTokenEstimator,
    );

    assert!(!report.prompt.contains("token=abc123"));
    assert!(report.prompt.contains("token=[REDACTED_SECRET]"));
    assert!(report.prompt.contains("tool_search"));
    assert!(report.sections.iter().any(|section| {
        section.kind == PromptSectionKind::Persona
            && section.source == PromptSourceKind::Runtime
            && section.title == "Agent loop base system"
    }));
    assert!(report.sections.iter().any(|section| {
        section.kind == PromptSectionKind::ToolGuidance
            && section.source == PromptSourceKind::Tooling
            && section.content.contains("Deferred tools")
            && section.content.contains("tool_call")
    }));
    assert!(report.estimated_tokens > 0);
}
