// SPDX-License-Identifier: GPL-3.0-only

use ikaros_execution::harness::{SkillDescriptor, SkillDescriptorKind, ToolVisibility};

use super::super::super::terminal_inline;

pub(super) fn tool_descriptor_status_json(
    descriptor: &SkillDescriptor,
    visibility: Option<ToolVisibility>,
) -> serde_json::Value {
    let callable = matches!(
        visibility,
        Some(ToolVisibility::Direct | ToolVisibility::Deferred)
    ) && descriptor.kind == SkillDescriptorKind::ExecutableTool
        && !descriptor.disable_model_invocation;
    serde_json::json!({
        "name": terminal_inline(&descriptor.name),
        "kind": skill_descriptor_kind_name(&descriptor.kind),
        "callable": callable,
        "toolset": descriptor.toolset.as_str(),
        "risk": format!("{:?}", descriptor.risk_level),
        "mode": descriptor.execution_mode.as_str(),
        "provenance": descriptor.provenance.as_deref().map(terminal_inline),
        "support_files": descriptor.support_files.len(),
    })
}

pub(super) fn tool_descriptor_status_line(
    descriptor: &SkillDescriptor,
    visibility: Option<ToolVisibility>,
) -> String {
    let callable = matches!(
        visibility,
        Some(ToolVisibility::Direct | ToolVisibility::Deferred)
    ) && descriptor.kind == SkillDescriptorKind::ExecutableTool
        && !descriptor.disable_model_invocation;
    format!(
        "{} kind={} callable={} toolset={} risk={:?} mode={} provenance={} support_files={}",
        descriptor.name,
        skill_descriptor_kind_name(&descriptor.kind),
        callable,
        descriptor.toolset,
        descriptor.risk_level,
        descriptor.execution_mode.as_str(),
        descriptor.provenance.as_deref().unwrap_or("-"),
        descriptor.support_files.len()
    )
}

pub(super) fn tool_descriptor_short_line(
    descriptor: &SkillDescriptor,
    visibility: Option<ToolVisibility>,
) -> String {
    let callable = matches!(
        visibility,
        Some(ToolVisibility::Direct | ToolVisibility::Deferred)
    ) && descriptor.kind == SkillDescriptorKind::ExecutableTool
        && !descriptor.disable_model_invocation;
    format!(
        "{} ({}, callable={})",
        descriptor.name,
        skill_descriptor_kind_name(&descriptor.kind),
        callable
    )
}

fn skill_descriptor_kind_name(kind: &SkillDescriptorKind) -> &'static str {
    match kind {
        SkillDescriptorKind::ExecutableTool => "executable_tool",
        SkillDescriptorKind::PromptSkill => "prompt_skill",
    }
}
