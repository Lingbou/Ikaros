// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    shell::{run_shell, validate_test_command},
    support::input_string,
};
use async_trait::async_trait;
use ikaros_coding::{
    ChangePlanner, CodeReviewAssistant, DiffSummarizer, GuardedPatchApplier, PatchIterationPlanner,
    RepoScanner, TestFailureAnalysis, TestFailureAnalyzer, TestRunnerPlan,
};
use ikaros_core::{Result, RiskLevel};
use ikaros_harness::{PolicyRequest, Skill, SkillContext, SkillOutput};
use serde_json::json;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct TaskSummarizeSkill;

#[async_trait]
impl Skill for TaskSummarizeSkill {
    fn name(&self) -> &'static str {
        "task_summarize"
    }

    fn description(&self) -> &'static str {
        "Summarize a task request."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["task"], "properties": {"task": {"type": "string"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        let task = input_string(&input, "task")?;
        Ok(SkillOutput::new(
            "task summarized",
            json!({"summary": format!("Task requires planning, policy evaluation, audited skill execution, and final explanation: {task}")}),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct RepoScanSkill;

#[async_trait]
impl Skill for RepoScanSkill {
    fn name(&self) -> &'static str {
        "repo_scan"
    }

    fn description(&self) -> &'static str {
        "Build a lightweight repo map."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object"})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, _input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let repo = RepoScanner::new(&ctx.session.sandbox.workspace_root).scan()?;
        Ok(SkillOutput::new("repo scanned", json!(repo)))
    }
}

#[derive(Debug, Clone)]
pub struct RunTestsSkill;

#[async_trait]
impl Skill for RunTestsSkill {
    fn name(&self) -> &'static str {
        "run_tests"
    }

    fn description(&self) -> &'static str {
        "Run an explicit test command or infer test commands from the repo map."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {"command": {"type": "string"}, "infer": {"type": "boolean"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::ShellRead
    }

    fn policy_request(&self, input: &serde_json::Value, _workspace_root: &Path) -> PolicyRequest {
        if input
            .get("infer")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            return PolicyRequest {
                action: self.name().into(),
                risk: RiskLevel::SafeRead,
                path: None,
                command: None,
                is_write: false,
            };
        }
        let command = input
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("cargo test --workspace --all-features");
        let risk = if crate::shell::is_allowed_test_command(command) {
            self.risk_level()
        } else {
            RiskLevel::Destructive
        };
        PolicyRequest {
            action: self.name().into(),
            risk,
            path: None,
            command: Some(command.into()),
            is_write: false,
        }
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        if input
            .get("infer")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            let repo = RepoScanner::new(&ctx.session.sandbox.workspace_root).scan()?;
            return Ok(SkillOutput::new(
                "test commands inferred",
                json!(TestRunnerPlan::infer(&repo)),
            ));
        }
        let command = input
            .get("command")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("cargo test --workspace --all-features");
        validate_test_command(command)?;
        let output = run_shell(command, &ctx.session).await?;
        let analysis =
            TestFailureAnalyzer::analyze(command, output.status, &output.stdout, &output.stderr);
        Ok(SkillOutput::new(
            "test command completed",
            json!({"command": command, "output": output, "analysis": analysis}),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct CodeEditGuardedSkill;

#[async_trait]
impl Skill for CodeEditGuardedSkill {
    fn name(&self) -> &'static str {
        "code_edit_guarded"
    }

    fn description(&self) -> &'static str {
        "Plan or apply a guarded unified diff after harness approval."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "required": ["objective"], "properties": {"objective": {"type": "string"}, "diff": {"type": "string"}, "plan_only": {"type": "boolean"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::LocalWrite
    }

    fn policy_request(&self, input: &serde_json::Value, _workspace_root: &Path) -> PolicyRequest {
        let plan_only = input
            .get("plan_only")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        PolicyRequest {
            action: self.name().into(),
            risk: if plan_only {
                RiskLevel::SafeRead
            } else {
                self.risk_level()
            },
            path: None,
            command: None,
            is_write: !plan_only,
        }
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let objective = input_string(&input, "objective")?;
        let repo = RepoScanner::new(&ctx.session.sandbox.workspace_root).scan()?;
        let plan = ChangePlanner::plan(objective, &repo);
        let diff_summary = input
            .get("diff")
            .and_then(serde_json::Value::as_str)
            .map(DiffSummarizer::summarize);
        let apply_report = if input
            .get("plan_only")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            None
        } else if let Some(diff) = input.get("diff").and_then(serde_json::Value::as_str) {
            if diff.trim().is_empty() {
                None
            } else {
                Some(GuardedPatchApplier::apply_unified_diff(
                    &ctx.session.sandbox.workspace_root,
                    diff,
                )?)
            }
        } else {
            None
        };
        let summary = if apply_report.is_some() {
            "guarded code edit applied"
        } else {
            "guarded code edit plan prepared"
        };
        Ok(SkillOutput::new(
            summary,
            json!({"plan": plan, "diff_summary": diff_summary, "apply_report": apply_report}),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct CodeReviewSkill;

#[async_trait]
impl Skill for CodeReviewSkill {
    fn name(&self) -> &'static str {
        "code_review"
    }

    fn description(&self) -> &'static str {
        "Generate a structured review report from a unified diff and optional test analysis."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {"diff": {"type": "string"}, "test_analysis": {"type": "object"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, _ctx: SkillContext) -> Result<SkillOutput> {
        let diff = input
            .get("diff")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let test_analysis = input
            .get("test_analysis")
            .filter(|value| !value.is_null())
            .map(|value| serde_json::from_value::<TestFailureAnalysis>(value.clone()))
            .transpose()?;
        let report = CodeReviewAssistant::review(diff, test_analysis);
        Ok(SkillOutput::new("code review complete", json!(report)))
    }
}

#[derive(Debug, Clone)]
pub struct CodeIterateSkill;

#[async_trait]
impl Skill for CodeIterateSkill {
    fn name(&self) -> &'static str {
        "code_iterate"
    }

    fn description(&self) -> &'static str {
        "Plan the next guarded patch iteration from a review report or unified diff."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({"type": "object", "properties": {"objective": {"type": "string"}, "diff": {"type": "string"}, "test_analysis": {"type": "object"}}})
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let objective = input
            .get("objective")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("prepare next guarded patch iteration");
        let diff = input
            .get("diff")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let test_analysis = input
            .get("test_analysis")
            .filter(|value| !value.is_null())
            .map(|value| serde_json::from_value::<TestFailureAnalysis>(value.clone()))
            .transpose()?;
        let review = CodeReviewAssistant::review(diff, test_analysis);
        let repo = RepoScanner::new(&ctx.session.sandbox.workspace_root).scan()?;
        let iteration = PatchIterationPlanner::plan(objective, &review, &repo);
        Ok(SkillOutput::new(
            "patch iteration plan prepared",
            json!({"review": review, "iteration": iteration}),
        ))
    }
}
