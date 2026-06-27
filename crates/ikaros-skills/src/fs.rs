// SPDX-License-Identifier: GPL-3.0-only

use crate::support::input_path;
use async_trait::async_trait;
use ikaros_core::{IkarosError, Result, RiskLevel};
use ikaros_tools::{Skill, SkillContext, SkillOutput};
use serde_json::json;

#[derive(Debug, Clone)]
pub struct FsReadSkill;

#[async_trait]
impl Skill for FsReadSkill {
    fn name(&self) -> &'static str {
        "fs_read"
    }

    fn description(&self) -> &'static str {
        "Read a UTF-8 file inside the active workspace."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let path = input_path(&input, &ctx.session.sandbox.workspace_root)?;
        let content = ctx.session.env.read_to_string(&path).await?;
        Ok(SkillOutput::new(
            format!("read {}", path.display()),
            json!({"path": path, "content": ikaros_core::redact_secrets(&content)}),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct FsWriteGuardedSkill;

#[async_trait]
impl Skill for FsWriteGuardedSkill {
    fn name(&self) -> &'static str {
        "fs_write_guarded"
    }

    fn description(&self) -> &'static str {
        "Write a UTF-8 file after harness policy and approval allow it."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["path", "content"], "properties": {"path": {"type": "string"}, "content": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::LocalWrite
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let path = input_path(&input, &ctx.session.sandbox.workspace_root)?;
        let content = input
            .get("content")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| IkarosError::Message("content is required".into()))?;
        ctx.session
            .env
            .write_string(&path, content.to_owned())
            .await?;
        Ok(SkillOutput::new(
            format!("wrote {}", path.display()),
            json!({"path": path}),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct ListDirSkill;

#[async_trait]
impl Skill for ListDirSkill {
    fn name(&self) -> &'static str {
        "list_dir"
    }

    fn description(&self) -> &'static str {
        "List directory entries."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["path"], "properties": {"path": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let path = input_path(&input, &ctx.session.sandbox.workspace_root)?;
        let entries = ctx.session.env.read_dir(&path).await?;
        Ok(SkillOutput::new(
            format!("listed {}", path.display()),
            json!({"path": path, "entries": entries}),
        ))
    }
}
