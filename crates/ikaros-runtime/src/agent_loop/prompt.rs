// SPDX-License-Identifier: GPL-3.0-only

use super::types::AgentLoopToolDefinition;
use ikaros_context::{
    PromptBuildReport, PromptBuilder, PromptSectionKind, PromptSourceKind, TokenEstimator,
};
use ikaros_core::{ResolvedAgentProfile, Result, redact_json, redact_secrets};
use ikaros_harness::{SkillRegistry, ToolsetSelection};
use ikaros_models::ModelToolDefinition;

pub fn agent_toolset_selection(agent: &ResolvedAgentProfile) -> Result<ToolsetSelection> {
    if agent.profile.toolsets.is_empty() {
        return Ok(ToolsetSelection::default());
    }
    ToolsetSelection::from_names(agent.profile.toolsets.iter())
}

pub fn agent_loop_tool_definitions(
    registry: &SkillRegistry,
    toolsets: &ToolsetSelection,
) -> Vec<AgentLoopToolDefinition> {
    let tool_registry = registry.tool_registry();
    let mut definitions = tool_registry
        .model_visible_names_for(toolsets)
        .into_iter()
        .filter_map(|name| {
            let skill = tool_registry.get(&name)?;
            let descriptor = skill.descriptor();
            Some(AgentLoopToolDefinition {
                name,
                description: descriptor.description,
                input_schema: redact_json(descriptor.input_schema),
                risk: descriptor.risk_level,
                execution_mode: descriptor.execution_mode,
                timeout_ms: descriptor.timeout_ms,
            })
        })
        .collect::<Vec<_>>();
    definitions.sort_by(|left, right| left.name.cmp(&right.name));
    definitions
}

pub(super) fn build_agent_loop_system_prompt(
    system_prompt: &str,
    tools: &[AgentLoopToolDefinition],
    estimator: &dyn TokenEstimator,
) -> PromptBuildReport {
    add_agent_loop_tool_guidance(
        PromptBuilder::new(estimator).add_section(
            PromptSectionKind::Persona,
            "Agent loop base system",
            system_prompt,
            PromptSourceKind::Runtime,
            100,
        ),
        tools,
    )
    .build()
}

pub(super) fn agent_loop_system_messages_for_model(
    prompt_report: &PromptBuildReport,
    split_system_prompts: &[String],
    tools: &[AgentLoopToolDefinition],
    estimator: &dyn TokenEstimator,
) -> Vec<String> {
    let mut split_system_prompts = split_system_prompts
        .iter()
        .map(|message| redact_secrets(message))
        .filter(|message| !message.trim().is_empty())
        .collect::<Vec<_>>();
    if split_system_prompts.is_empty() {
        return prompt_report.system_messages_for_prompt_cache();
    }

    let tool_prompt = build_agent_loop_tool_guidance_prompt(tools, estimator).prompt;
    if !tool_prompt.trim().is_empty() {
        split_system_prompts[0].push_str("\n\n");
        split_system_prompts[0].push_str(&tool_prompt);
    }
    split_system_prompts
}

fn build_agent_loop_tool_guidance_prompt(
    tools: &[AgentLoopToolDefinition],
    estimator: &dyn TokenEstimator,
) -> PromptBuildReport {
    add_agent_loop_tool_guidance(PromptBuilder::new(estimator), tools).build()
}

fn add_agent_loop_tool_guidance<'a>(
    mut builder: PromptBuilder<'a>,
    tools: &[AgentLoopToolDefinition],
) -> PromptBuilder<'a> {
    let tool_manifest = serde_json::to_string(tools).unwrap_or_else(|_| "[]".into());
    builder = builder.add_section(
        PromptSectionKind::ToolGuidance,
        "Tool-call protocol",
        "Use provider-native tool calls when the provider supports them. Otherwise the only accepted JSON fallback is exactly {\"tool_calls\":[{\"id\":\"optional_call_id\",\"name\":\"tool_name\",\"input\":{}}]} for tool use or {\"final_answer\":\"...\"} when done. Do not use alternate keys such as tools, calls, function_call, args, arguments, answer, or response.",
        PromptSourceKind::Tooling,
        100,
    );
    if has_deferred_tool_bridge(tools) {
        builder = builder.add_section(
            PromptSectionKind::ToolGuidance,
            "Deferred tool disclosure",
            "Deferred tools are not callable by name until disclosed in this session. Use tool_search to find deferred tools or tool_describe to disclose one known deferred tool, then call the disclosed target through tool_call. Do not call tool_call for a deferred target that has not been disclosed.",
            PromptSourceKind::Tooling,
            100,
        );
    }
    builder.add_section(
        PromptSectionKind::ToolGuidance,
        "Available tools",
        format!("Available tools: {tool_manifest}"),
        PromptSourceKind::Tooling,
        95,
    )
}

fn has_deferred_tool_bridge(tools: &[AgentLoopToolDefinition]) -> bool {
    ["tool_search", "tool_describe", "tool_call"]
        .into_iter()
        .all(|required| tools.iter().any(|tool| tool.name == required))
}

pub(super) fn model_tool_definitions(
    tools: &[AgentLoopToolDefinition],
) -> Vec<ModelToolDefinition> {
    tools
        .iter()
        .map(|tool| ModelToolDefinition {
            name: tool.name.clone(),
            description: tool.description.clone(),
            input_schema: tool.input_schema.clone(),
        })
        .collect()
}
