// SPDX-License-Identifier: GPL-3.0-only

use super::types::AgentLoopToolDefinition;
use ikaros_core::{redact_json, redact_secrets};
use ikaros_harness::SkillRegistry;
use ikaros_models::ModelToolDefinition;

pub fn agent_loop_tool_definitions(registry: &SkillRegistry) -> Vec<AgentLoopToolDefinition> {
    let mut definitions = registry
        .model_visible_names()
        .into_iter()
        .filter_map(|name| {
            let skill = registry.get(&name)?;
            let descriptor = skill.descriptor();
            Some(AgentLoopToolDefinition {
                name,
                description: descriptor.description,
                input_schema: redact_json(descriptor.input_schema),
                risk: descriptor.risk_level,
            })
        })
        .collect::<Vec<_>>();
    definitions.sort_by(|left, right| left.name.cmp(&right.name));
    definitions
}

pub(super) fn render_agent_loop_system_prompt(
    system_prompt: &str,
    tools: &[AgentLoopToolDefinition],
) -> String {
    let tool_manifest = serde_json::to_string(tools).unwrap_or_else(|_| "[]".into());
    redact_secrets(&format!(
        "{system_prompt}\n\nWhen tool use is needed, respond only as JSON: {{\"tool_calls\":[{{\"name\":\"tool_name\",\"input\":{{}}}}]}}. When done, respond as JSON: {{\"final_answer\":\"...\"}}.\nAvailable tools: {tool_manifest}"
    ))
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
