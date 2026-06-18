// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    CodingSessionConfig,
    shell::{run_shell, validate_test_command},
    support::input_string,
};
use async_trait::async_trait;
use ikaros_coding::{
    ChangePlanner, CodeReviewAssistant, CodingMode, CodingModeCapabilities,
    CodingPermissionProfile, CodingTurnContext, CodingTurnContextInput, CodingTurnInput,
    DeterministicCodingRuntime, DiffSummarizer, GuardedPatchApplier, PatchIterationPlanner,
    RepoScanner, TestCommand, TestFailureAnalysis, TestFailureAnalyzer, TestRunnerPlan,
};
use ikaros_core::{IkarosError, Result, RiskLevel, redact_secrets};
use ikaros_harness::{PolicyRequest, Skill, SkillContext, SkillOutput};
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSource, SessionEntry, SessionEntryKind, SessionRecord,
};
use serde_json::{Value, json};
use std::path::Path;

mod model_loop;

use model_loop::{load_workspace_coding_instructions, run_provider_coding_loop};

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
                Some(
                    GuardedPatchApplier::apply_unified_diff_with_env(
                        &ctx.session.sandbox.workspace_root,
                        diff,
                        ctx.session.env.as_ref(),
                    )
                    .await?,
                )
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

#[derive(Debug, Clone)]
pub struct CodeWorkflowSkill {
    coding_session: Option<CodingSessionConfig>,
}

impl CodeWorkflowSkill {
    pub fn new(coding_session: Option<CodingSessionConfig>) -> Self {
        Self { coding_session }
    }
}

#[async_trait]
impl Skill for CodeWorkflowSkill {
    fn name(&self) -> &'static str {
        "code_workflow"
    }

    fn description(&self) -> &'static str {
        "Run the controlled coding workflow: repo map, plan, candidate patch review, test evidence, iteration plan, and final report."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["objective"],
            "properties": {
                "objective": {"type": "string"},
                "mode": {
                    "type": "string",
                    "enum": ["plan", "edit", "review", "test", "self_modify"]
                },
                "diff": {"type": "string"},
                "apply_patch": {"type": "boolean"},
                "run_tests": {"type": "boolean"},
                "model_loop": {"type": "boolean"},
                "max_iterations": {"type": "integer", "minimum": 1, "maximum": 8},
                "model_token_budget": {"type": "integer", "minimum": 1},
                "test_analysis": {"type": "object"},
                "test_commands": {
                    "type": "array",
                    "items": {
                        "oneOf": [
                            {"type": "string"},
                            {
                                "type": "object",
                                "required": ["command"],
                                "properties": {
                                    "command": {"type": "string"},
                                    "reason": {"type": "string"}
                                }
                            }
                        ]
                    }
                },
                "instructions": {"type": "array", "items": {"type": "string"}},
                "session_id": {"type": "string"},
                "turn_id": {"type": "string"}
            }
        })
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::SafeRead
    }

    fn policy_request(&self, input: &serde_json::Value, _workspace_root: &Path) -> PolicyRequest {
        let mode = parse_coding_mode_lossy(input);
        let apply_patch = code_workflow_apply_patch_requested(input);
        let runs_tests = code_workflow_runs_tests(input);
        let model_loop = code_workflow_model_loop_requested(input);
        let capabilities = CodingModeCapabilities::for_mode(mode);
        let invalid_request = capabilities
            .validate_request(apply_patch, runs_tests)
            .is_err();
        let writes = apply_patch && capabilities.can_apply_patch;
        PolicyRequest {
            action: self.name().into(),
            risk: if capabilities.requires_self_modify_boundary {
                RiskLevel::SelfModify
            } else if invalid_request {
                RiskLevel::Destructive
            } else if writes {
                RiskLevel::LocalWrite
            } else if runs_tests {
                RiskLevel::ShellRead
            } else if model_loop {
                RiskLevel::Network
            } else {
                self.risk_level()
            },
            path: None,
            command: None,
            is_write: writes,
        }
    }

    async fn execute(&self, input: serde_json::Value, ctx: SkillContext) -> Result<SkillOutput> {
        let objective = input_string(&input, "objective")?;
        let mode = parse_coding_mode(&input)?;
        let diff = input
            .get("diff")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned);
        let apply_patch = input
            .get("apply_patch")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let model_loop = code_workflow_model_loop_requested(&input);
        let max_iterations = parse_max_iterations(&input)?;
        let model_token_budget = parse_model_token_budget(&input)?;
        let runs_tests = code_workflow_runs_tests(&input);
        CodingModeCapabilities::for_mode(mode).validate_request(apply_patch, runs_tests)?;
        let mut test_matrix = Vec::new();
        let mut test_analysis = input
            .get("test_analysis")
            .filter(|value| !value.is_null())
            .map(|value| serde_json::from_value::<TestFailureAnalysis>(value.clone()))
            .transpose()?;
        let test_commands = parse_test_commands(input.get("test_commands"))?;
        if runs_tests && !model_loop {
            test_matrix = run_coding_test_matrix(&test_commands, &ctx).await?;
            test_analysis = primary_test_analysis(&test_matrix);
        }
        let mut instructions = load_workspace_coding_instructions(&ctx).await?;
        instructions.extend(parse_string_array(
            input.get("instructions"),
            "instructions",
        )?);
        let context = CodingTurnContext::from_workspace_with_process(
            CodingTurnContextInput {
                workspace_root: ctx.session.sandbox.workspace_root.clone(),
                objective,
                mode,
                instructions,
                test_commands,
                session_id: input
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
                turn_id: input
                    .get("turn_id")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
                permission_profile: CodingPermissionProfile::default(),
            },
            ctx.session.env.as_ref(),
        )
        .await?;
        let turn_input = CodingTurnInput {
            context,
            candidate_diff: diff,
            apply_patch,
            test_matrix,
            test_analysis,
        };
        let report = if model_loop {
            let coding_session = self.coding_session.as_ref().ok_or_else(|| {
                IkarosError::Message("model_loop requires a configured coding session".into())
            })?;
            let provider = coding_session.model_provider.clone().ok_or_else(|| {
                IkarosError::Message(
                    "model_loop requires a configured coding model provider".into(),
                )
            })?;
            run_provider_coding_loop(
                turn_input,
                provider,
                &ctx,
                runs_tests,
                max_iterations,
                model_token_budget,
                coding_session.cancellation.clone(),
            )
            .await?
        } else {
            DeterministicCodingRuntime
                .run_turn_with_env(turn_input, ctx.session.env.as_ref())
                .await?
        };
        persist_coding_turn_report(self.coding_session.as_ref(), &report)?;
        Ok(SkillOutput::new("coding turn completed", json!(report)))
    }
}

async fn run_coding_test_matrix(
    test_commands: &[TestCommand],
    ctx: &SkillContext,
) -> Result<Vec<TestFailureAnalysis>> {
    let commands = if test_commands.is_empty() {
        let repo = RepoScanner::new(&ctx.session.sandbox.workspace_root).scan()?;
        let inferred = TestRunnerPlan::infer(&repo);
        if inferred.is_empty() {
            return Err(IkarosError::Message(
                "no test command inferred for coding turn".into(),
            ));
        }
        inferred
    } else {
        test_commands.to_vec()
    };
    let mut matrix = Vec::with_capacity(commands.len());
    for command in commands {
        let output = run_shell(&command.command, &ctx.session).await?;
        matrix.push(TestFailureAnalyzer::analyze(
            command.command,
            output.status,
            &output.stdout,
            &output.stderr,
        ));
    }
    Ok(matrix)
}

fn bounded_redacted_text(text: &str, max_chars: usize) -> String {
    let redacted = redact_secrets(text);
    let mut chars = redacted.chars();
    let mut output = chars.by_ref().take(max_chars.max(1)).collect::<String>();
    if chars.next().is_some() {
        output.push_str("\n[TRUNCATED]");
    }
    output
}

fn primary_test_analysis(test_matrix: &[TestFailureAnalysis]) -> Option<TestFailureAnalysis> {
    test_matrix
        .iter()
        .find(|analysis| analysis.status != 0)
        .or_else(|| test_matrix.first())
        .cloned()
}

pub(crate) fn persist_coding_turn_report(
    config: Option<&CodingSessionConfig>,
    report: &ikaros_coding::CodingTurnReport,
) -> Result<()> {
    let Some(config) = config else {
        return Ok(());
    };
    let mut session = SessionRecord::new(config.session_id.clone(), config.source.clone());
    session.agent_id = config.agent_id.clone();
    session.workspace = config.workspace.clone();
    let mut writer = config.store.begin_turn(&session, &config.turn_id)?;
    for event in &report.events {
        let payload = coding_event_payload(event)?;
        writer.append_agent_event(&AgentEvent::new(
            config.session_id.clone(),
            config.turn_id.clone(),
            None,
            AgentEventSource::Tool,
            AgentEventKind::CodingTurn,
            payload.clone(),
        ))?;
        let mut entry = SessionEntry::new(config.session_id.clone(), SessionEntryKind::Custom);
        entry.turn_id = Some(config.turn_id.clone());
        entry.visible_text = Some(event.summary.clone());
        entry.payload = payload;
        writer.append_entry(&entry)?;
    }
    writer.commit()
}

fn coding_event_payload(event: &ikaros_coding::CodingTurnEvent) -> Result<Value> {
    Ok(json!({
        "kind": serde_json::to_value(event.kind)?,
        "summary": event.summary,
        "payload": event.payload,
    }))
}

fn parse_coding_mode(input: &serde_json::Value) -> Result<CodingMode> {
    let mode = input
        .get("mode")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("plan");
    match mode {
        "plan" => Ok(CodingMode::Plan),
        "edit" => Ok(CodingMode::Edit),
        "review" => Ok(CodingMode::Review),
        "test" => Ok(CodingMode::Test),
        "self_modify" => Ok(CodingMode::SelfModify),
        other => Err(IkarosError::Message(format!(
            "unsupported coding mode: {other}"
        ))),
    }
}

fn parse_coding_mode_lossy(input: &serde_json::Value) -> CodingMode {
    parse_coding_mode(input).unwrap_or(CodingMode::Plan)
}

fn code_workflow_apply_patch_requested(input: &serde_json::Value) -> bool {
    let apply_patch = input
        .get("apply_patch")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if code_workflow_model_loop_requested(input) {
        return apply_patch;
    }
    let has_diff = input
        .get("diff")
        .and_then(serde_json::Value::as_str)
        .map(|diff| !diff.trim().is_empty())
        .unwrap_or(false);
    apply_patch && has_diff
}

fn code_workflow_model_loop_requested(input: &serde_json::Value) -> bool {
    input
        .get("model_loop")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn code_workflow_runs_tests(input: &serde_json::Value) -> bool {
    input
        .get("run_tests")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn parse_max_iterations(input: &serde_json::Value) -> Result<usize> {
    let Some(value) = input.get("max_iterations") else {
        return Ok(1);
    };
    let raw = value
        .as_u64()
        .ok_or_else(|| IkarosError::Message("max_iterations must be a positive integer".into()))?;
    if raw == 0 || raw > 8 {
        return Err(IkarosError::Message(
            "max_iterations must be between 1 and 8".into(),
        ));
    }
    Ok(raw as usize)
}

fn parse_model_token_budget(input: &serde_json::Value) -> Result<Option<u32>> {
    let Some(value) = input.get("model_token_budget") else {
        return Ok(None);
    };
    let raw = value.as_u64().ok_or_else(|| {
        IkarosError::Message("model_token_budget must be a positive integer".into())
    })?;
    if raw == 0 || raw > u32::MAX as u64 {
        return Err(IkarosError::Message(
            "model_token_budget must be between 1 and u32::MAX".into(),
        ));
    }
    Ok(Some(raw as u32))
}

fn parse_test_commands(value: Option<&serde_json::Value>) -> Result<Vec<TestCommand>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let commands = value
        .as_array()
        .ok_or_else(|| IkarosError::Message("test_commands must be an array".into()))?;
    commands
        .iter()
        .enumerate()
        .map(|(index, command)| {
            if let Some(command) = command.as_str() {
                return Ok(TestCommand {
                    command: command.to_owned(),
                    reason: "explicit coding turn command".into(),
                });
            }
            let object = command.as_object().ok_or_else(|| {
                IkarosError::Message(format!("test_commands[{index}] must be a string or object"))
            })?;
            let command = object
                .get("command")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    IkarosError::Message(format!("test_commands[{index}].command is required"))
                })?;
            let reason = object
                .get("reason")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("explicit coding turn command");
            Ok(TestCommand {
                command: command.to_owned(),
                reason: reason.to_owned(),
            })
        })
        .collect()
}

fn parse_string_array(value: Option<&serde_json::Value>, field: &str) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let values = value
        .as_array()
        .ok_or_else(|| IkarosError::Message(format!("{field} must be an array")))?;
    values
        .iter()
        .enumerate()
        .map(|(index, value)| {
            value
                .as_str()
                .map(ToOwned::to_owned)
                .ok_or_else(|| IkarosError::Message(format!("{field}[{index}] must be a string")))
        })
        .collect()
}
