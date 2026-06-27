// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_core::IkarosPaths;
use ikaros_execution::harness::ExecutionSession;
use std::path::Path;

pub(crate) fn session_and_registry(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<(ExecutionSession, ikaros_execution::harness::SkillRegistry)> {
    Ok(ikaros_host::session_and_registry(
        paths,
        workspace,
        agent_override,
    )?)
}

pub(crate) fn print_skill_result(result: &ikaros_core::ToolResult) -> Result<()> {
    println!("ok: {}", result.ok);
    println!("summary: {}", result.summary);
    println!("{}", serde_json::to_string_pretty(&result.output)?);
    Ok(())
}

pub(crate) fn print_approval_hint(result: &ikaros_core::ToolResult) {
    if let Some(id) = result
        .output
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
    {
        println!("approval: {id}");
        println!("next: ikaros approval approve {id}");
    }
}
