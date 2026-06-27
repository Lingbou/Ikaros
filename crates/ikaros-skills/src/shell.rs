// SPDX-License-Identifier: GPL-3.0-only

use crate::support::input_string;
use async_trait::async_trait;
use ikaros_core::{IkarosError, Result, RiskLevel};
use ikaros_harness::{
    PolicyRequest, ProcessOutput, ProcessRequest, Skill, SkillContext, SkillOutput,
};
use ikaros_tools::SkillRuntimeSession;
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct ShellGuardedSkill;

#[async_trait]
impl Skill for ShellGuardedSkill {
    fn name(&self) -> &'static str {
        "shell_guarded"
    }

    fn description(&self) -> &'static str {
        "Run an allowlisted test/check command after policy evaluation."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["command"], "properties": {"command": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::ShellRead
    }

    fn policy_request(&self, input: &serde_json::Value, _workspace_root: &Path) -> PolicyRequest {
        let command = input
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let risk = if is_allowed_test_command(command) {
            RiskLevel::ShellRead
        } else {
            RiskLevel::Destructive
        };
        PolicyRequest {
            action: self.name().into(),
            risk: risk.clone(),
            path: None,
            command: Some(command.into()),
            is_write: matches!(
                risk,
                RiskLevel::LocalWrite | RiskLevel::ShellWrite | RiskLevel::DatabaseWrite
            ),
        }
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let command = input_string(&input, "command")?;
        validate_test_command(&command)?;
        let output = run_shell(&command, &ctx.session).await?;
        Ok(SkillOutput::new(
            format!("ran allowlisted command: {command}"),
            json!(output),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct GitStatusSkill;

#[async_trait]
impl Skill for GitStatusSkill {
    fn name(&self) -> &'static str {
        "git_status"
    }

    fn description(&self) -> &'static str {
        "Run git status --short."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::ShellRead
    }

    fn policy_request(&self, _input: &serde_json::Value, _workspace_root: &Path) -> PolicyRequest {
        PolicyRequest {
            action: self.name().into(),
            risk: self.risk_level(),
            path: None,
            command: Some("git status --short".into()),
            is_write: false,
        }
    }

    async fn execute(&self, _input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let output =
            run_program("git", vec!["status".into(), "--short".into()], &ctx.session).await?;
        Ok(SkillOutput::new("git status collected", json!(output)))
    }
}

#[derive(Debug, Clone)]
pub struct GitDiffSkill;

#[async_trait]
impl Skill for GitDiffSkill {
    fn name(&self) -> &'static str {
        "git_diff"
    }

    fn description(&self) -> &'static str {
        "Run git diff --stat or full diff."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {"stat": {"type": "boolean"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::ShellRead
    }

    fn policy_request(&self, input: &serde_json::Value, _workspace_root: &Path) -> PolicyRequest {
        let stat = input
            .get("stat")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        PolicyRequest {
            action: self.name().into(),
            risk: self.risk_level(),
            path: None,
            command: Some(if stat { "git diff --stat" } else { "git diff" }.into()),
            is_write: false,
        }
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let stat = input
            .get("stat")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let args = if stat {
            vec!["diff".into(), "--stat".into()]
        } else {
            vec!["diff".into()]
        };
        let output = run_program("git", args, &ctx.session).await?;
        Ok(SkillOutput::new("git diff collected", json!(output)))
    }
}

pub(crate) fn validate_test_command(command: &str) -> Result<()> {
    ikaros_coding::validate_test_command(command)
}

pub(crate) fn is_allowed_test_command(command: &str) -> bool {
    ikaros_coding::is_allowed_test_command(command)
}

pub(crate) async fn run_shell(
    command: &str,
    session: &SkillRuntimeSession,
) -> Result<ProcessOutput> {
    validate_test_command(command)?;
    let (program, args) = parse_allowlisted_command(command)?;
    run_program(&program, args, session).await
}

async fn run_program(
    program: &str,
    args: Vec<String>,
    session: &SkillRuntimeSession,
) -> Result<ProcessOutput> {
    session
        .env
        .run_process(ProcessRequest::program(
            program.to_string(),
            args,
            &session.sandbox.workspace_root,
        ))
        .await
}

fn parse_allowlisted_command(command: &str) -> Result<(String, Vec<String>)> {
    let parts = command
        .split_whitespace()
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    let Some((program, args)) = parts.split_first() else {
        return Err(IkarosError::Message("command is required".into()));
    };
    if parts
        .iter()
        .any(|part| part.as_bytes().contains(&0) || part.chars().any(char::is_control))
    {
        return Err(IkarosError::Message(
            "command must not contain control characters".into(),
        ));
    }
    Ok((program.clone(), args.to_vec()))
}
