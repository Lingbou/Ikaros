// SPDX-License-Identifier: GPL-3.0-only

use crate::support::{input_object, input_string};
use async_trait::async_trait;
use ikaros_core::{IkarosError, Result, RiskLevel, redact_secrets};
use ikaros_harness::{
    AuditEvent, Skill, SkillContext, SkillDescriptor, SkillDescriptorKind, SkillOutput,
    SkillRegistry, ToolVisibility, ToolsetSelection,
};
use serde_json::json;

#[derive(Clone)]
pub struct ToolSearchSkill {
    registry: SkillRegistry,
}

#[derive(Clone)]
pub struct ToolDescribeSkill {
    registry: SkillRegistry,
}

#[derive(Clone)]
pub struct ToolCallSkill {
    registry: SkillRegistry,
}

impl ToolSearchSkill {
    pub fn new(registry: SkillRegistry) -> Self {
        Self { registry }
    }
}

impl ToolDescribeSkill {
    pub fn new(registry: SkillRegistry) -> Self {
        Self { registry }
    }
}

impl ToolCallSkill {
    pub fn new(registry: SkillRegistry) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl Skill for ToolSearchSkill {
    fn name(&self) -> &'static str {
        "tool_search"
    }

    fn description(&self) -> &'static str {
        "Search deferred tool descriptors by name, description, or toolset."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 20}
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let query = input_string(&input, "query")?;
        let query = query.trim();
        if query.is_empty() {
            return Err(IkarosError::Message(
                "tool_search query must not be empty".into(),
            ));
        }
        let query = query.to_ascii_lowercase();
        let limit = input
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(10)
            .clamp(1, 20) as usize;
        let descriptors = deferred_descriptors(&self.registry, &ctx.toolsets)
            .into_iter()
            .filter(|descriptor| descriptor_matches(descriptor, &query))
            .take(limit)
            .collect::<Vec<_>>();
        ctx.session
            .disclose_deferred_tools(descriptors.iter().map(|descriptor| descriptor.name.clone()));
        let tools = descriptors
            .into_iter()
            .map(compact_descriptor_json)
            .collect::<Vec<_>>();
        Ok(SkillOutput::new(
            format!("found {} deferred tool(s)", tools.len()),
            json!({"tools": tools}),
        ))
    }
}

#[async_trait]
impl Skill for ToolDescribeSkill {
    fn name(&self) -> &'static str {
        "tool_describe"
    }

    fn description(&self) -> &'static str {
        "Describe a deferred tool, including its input schema and risk."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["name"], "properties": {"name": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let name = input_string(&input, "name")?;
        let descriptor = deferred_descriptor(&self.registry, &name, &ctx.toolsets)?;
        ctx.session.disclose_deferred_tool(name.clone());
        if descriptor.kind == SkillDescriptorKind::PromptSkill {
            let document = self.registry.prompt_skill(&name).ok_or_else(|| {
                IkarosError::Message(format!("prompt skill `{name}` has no instruction document"))
            })?;
            return Ok(SkillOutput::new(
                format!("described prompt skill: {name}"),
                json!({
                    "skill": {
                        "descriptor": document.descriptor,
                        "instructions": redact_secrets(&document.instructions),
                        "support_files": document.support_files.into_iter().map(|file| {
                            json!({
                                "path": file.path,
                                "content": redact_secrets(&file.content),
                                "truncated": file.truncated,
                            })
                        }).collect::<Vec<_>>(),
                    }
                }),
            ));
        }
        Ok(SkillOutput::new(
            format!("described deferred tool: {name}"),
            json!({"tool": descriptor}),
        ))
    }
}

#[async_trait]
impl Skill for ToolCallSkill {
    fn name(&self) -> &'static str {
        "tool_call"
    }

    fn description(&self) -> &'static str {
        "Call a deferred tool by name after tool_search or tool_describe has disclosed it. The target tool still runs through harness policy, approval, and audit."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["name", "input"],
            "properties": {
                "name": {"type": "string"},
                "input": {"type": "object"}
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let name = input_string(&input, "name")?;
        let target_input = input_object(&input, "input")?;
        let descriptor = deferred_descriptor(&self.registry, &name, &ctx.toolsets)?;
        if descriptor.kind == SkillDescriptorKind::PromptSkill {
            return Err(IkarosError::Message(format!(
                "prompt skill `{name}` is not executable; use tool_describe to read its instructions"
            )));
        }
        if !ctx.session.is_deferred_tool_disclosed(&name) {
            return Err(IkarosError::Message(format!(
                "deferred tool `{name}` has not been disclosed; call tool_search or tool_describe first"
            )));
        }
        ctx.session.audit.append(AuditEvent::new(
            "deferred_tool_invocation",
            None,
            format!("deferred tool invocation: {name}"),
            json!({
                "bridge_tool": self.name(),
                "target_tool": &descriptor.name,
                "target_toolset": descriptor.toolset,
                "target_kind": descriptor.kind,
                "target_risk": descriptor.risk_level,
                "target_execution_mode": descriptor.execution_mode,
                "target_callable": descriptor.kind == SkillDescriptorKind::ExecutableTool
                    && !descriptor.disable_model_invocation,
                "target_provenance": descriptor.provenance,
            }),
        )?)?;
        let result = ctx
            .session
            .execute_skill(
                &self.registry,
                &name,
                serde_json::Value::Object(target_input),
            )
            .await?;
        Ok(SkillOutput::new(
            result.summary.clone(),
            json!({
                "tool": name,
                "result": result,
            }),
        ))
    }
}

fn deferred_descriptors(
    registry: &SkillRegistry,
    allowed_toolsets: &ToolsetSelection,
) -> Vec<SkillDescriptor> {
    let mut descriptors = registry
        .descriptors()
        .into_iter()
        .filter(|descriptor| {
            registry.visibility_for(&descriptor.name, allowed_toolsets)
                == Some(ToolVisibility::Deferred)
        })
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.name.cmp(&right.name));
    descriptors
}

fn deferred_descriptor(
    registry: &SkillRegistry,
    name: &str,
    allowed_toolsets: &ToolsetSelection,
) -> Result<SkillDescriptor> {
    if let Some(disabled) = disabled_toolset_descriptor(registry, name, allowed_toolsets) {
        return Err(IkarosError::Message(format!(
            "deferred tool `{name}` requires disabled toolset `{}`",
            disabled.toolset
        )));
    }
    deferred_descriptors(registry, allowed_toolsets)
        .into_iter()
        .find(|descriptor| descriptor.name == name)
        .ok_or_else(|| IkarosError::Message(format!("deferred tool not found: {name}")))
}

fn disabled_toolset_descriptor(
    registry: &SkillRegistry,
    name: &str,
    allowed_toolsets: &ToolsetSelection,
) -> Option<SkillDescriptor> {
    registry.descriptors().into_iter().find(|descriptor| {
        descriptor.name == name
            && registry.visibility_for(&descriptor.name, allowed_toolsets)
                == Some(ToolVisibility::Disabled)
    })
}

fn descriptor_matches(descriptor: &SkillDescriptor, query: &str) -> bool {
    query.is_empty()
        || descriptor.name.to_ascii_lowercase().contains(query)
        || descriptor.description.to_ascii_lowercase().contains(query)
        || descriptor.toolset.as_str().contains(query)
}

fn compact_descriptor_json(descriptor: SkillDescriptor) -> serde_json::Value {
    json!({
        "name": descriptor.name,
        "description": descriptor.description,
        "risk": descriptor.risk_level,
        "kind": descriptor.kind,
        "callable": descriptor.kind == SkillDescriptorKind::ExecutableTool
            && !descriptor.disable_model_invocation,
        "toolset": descriptor.toolset,
        "execution_mode": descriptor.execution_mode,
        "timeout_ms": descriptor.timeout_ms,
        "provenance": descriptor.provenance,
        "support_file_count": descriptor.support_files.len(),
        "support_files": descriptor.support_files,
    })
}
