// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    ChangePlan, ChangePlanner, CodeReviewAssistant, CodingMode, CodingTurnContext,
    GuardedPatchApplier, PatchApplyReport, PatchFailure, PatchIterationPlan, PatchIterationPlanner,
    RepoMap, RepoScanner, ReviewReport, TestCommand, TestFailureAnalysis, TestRunnerPlan,
    TurnDiffSummary, TurnDiffTracker,
};
use ikaros_core::{Result, redact_json, redact_secrets};
use ikaros_harness::FileSystem as ExecutionFileSystem;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub trait CodingRuntime {
    fn run_turn(&self, input: CodingTurnInput) -> Result<CodingTurnReport>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodingTurnInput {
    pub context: CodingTurnContext,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_diff: Option<String>,
    #[serde(default)]
    pub apply_patch: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_matrix: Vec<TestFailureAnalysis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_analysis: Option<TestFailureAnalysis>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodingTurnReport {
    pub context: CodingTurnContext,
    pub repo: RepoMap,
    pub change_plan: ChangePlan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_apply_report: Option<PatchApplyReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch_failure: Option<PatchFailure>,
    pub turn_diff: CodingTurnDiffReport,
    pub suggested_tests: Vec<TestCommand>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_matrix: Vec<TestFailureAnalysis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_analysis: Option<TestFailureAnalysis>,
    pub review: ReviewReport,
    pub iteration: PatchIterationPlan,
    pub loop_report: CodingLoopReport,
    pub events: Vec<CodingTurnEvent>,
    pub final_report: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingLoopReport {
    pub status: CodingLoopStatus,
    pub iterations: usize,
    pub max_iterations: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodingLoopStatus {
    Passed,
    AwaitingFollowUpPatch,
    PatchFailed,
    ReviewBlocked,
    BudgetExceeded,
    Cancelled,
    ApprovalPending,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingTurnDiffReport {
    pub summary: TurnDiffSummary,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unified_diff: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CodingTurnEvent {
    pub kind: CodingTurnEventKind,
    pub summary: String,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodingTurnEventKind {
    ContextPrepared,
    GitBaselineCaptured,
    LoopIterationStarted,
    ModelRequestPrepared,
    ModelResponseReceived,
    ModelResponseInvalid,
    ModelBudgetExceeded,
    CodingLoopCancelled,
    RepoScanned,
    PlanPrepared,
    PatchSkipped,
    PatchApplied,
    PatchFailed,
    DiffUpdated,
    TestEvidenceRecorded,
    ReviewStarted,
    ReviewFinding,
    ReviewCompleted,
    IterationPlanned,
    LoopTerminated,
    FinalReportPrepared,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicCodingRuntime;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockModelCodingInput {
    pub context: CodingTurnContext,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turns: Vec<MockModelCodingTurn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MockModelCodingTurn {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate_diff: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_matrix: Vec<TestFailureAnalysis>,
}

#[derive(Debug, Clone, Copy)]
pub struct MockModelCodingRuntime {
    pub max_iterations: usize,
}

impl Default for MockModelCodingRuntime {
    fn default() -> Self {
        Self { max_iterations: 4 }
    }
}

impl CodingRuntime for DeterministicCodingRuntime {
    fn run_turn(&self, input: CodingTurnInput) -> Result<CodingTurnReport> {
        let mut turn = PreparedCodingTurn::new(input)?;

        if should_apply_candidate_patch(&turn.input) {
            if let Some(diff) = turn.input.candidate_diff.as_deref() {
                match GuardedPatchApplier::apply_unified_diff(
                    &turn.input.context.workspace_root,
                    diff,
                ) {
                    Ok(report) => turn.record_patch_applied(report)?,
                    Err(error) => turn.record_patch_failed(PatchFailure::from_error(error)),
                }
            }
        } else {
            turn.record_patch_skipped();
        }

        Ok(turn.finish())
    }
}

impl DeterministicCodingRuntime {
    pub async fn run_turn_with_env(
        &self,
        input: CodingTurnInput,
        file_system: &dyn ExecutionFileSystem,
    ) -> Result<CodingTurnReport> {
        let mut turn = PreparedCodingTurn::new(input)?;

        if should_apply_candidate_patch(&turn.input) {
            if let Some(diff) = turn.input.candidate_diff.as_deref() {
                match GuardedPatchApplier::apply_unified_diff_with_env_checked(
                    &turn.input.context.workspace_root,
                    diff,
                    file_system,
                )
                .await
                {
                    Ok(report) => turn.record_patch_applied(report)?,
                    Err(failure) => turn.record_patch_failed(failure),
                }
            }
        } else {
            turn.record_patch_skipped();
        }

        Ok(turn.finish())
    }
}

impl MockModelCodingRuntime {
    pub fn run_scripted_turns(&self, input: MockModelCodingInput) -> Result<CodingTurnReport> {
        let max_iterations = self.max_iterations.max(1).min(input.turns.len().max(1));
        let mut events = Vec::new();
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::ContextPrepared,
            format!("coding context prepared for {:?} mode", input.context.mode),
            json!({
                "workspace_root": input.context.workspace_root,
                "session_id": input.context.session_id,
                "turn_id": input.context.turn_id,
                "git": input.context.git,
            }),
        ));
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::GitBaselineCaptured,
            "git baseline captured for mock-model coding turn",
            json!({
                "git_root": input.context.git.git_root,
                "head": input.context.git.head,
                "branch": input.context.git.branch,
                "detached": input.context.git.detached,
                "dirty": input.context.git.dirty,
                "has_staged_changes": input.context.git.has_staged_changes,
                "has_unstaged_changes": input.context.git.has_unstaged_changes,
                "has_untracked_files": input.context.git.has_untracked_files,
            }),
        ));

        let repo = RepoScanner::new(&input.context.workspace_root).scan()?;
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::RepoScanned,
            format!(
                "repo scanned: {} file(s), {} package file(s)",
                repo.files.len(),
                repo.package_files.len()
            ),
            json!({
                "files": repo.files.len(),
                "package_files": repo.package_files.len(),
            }),
        ));
        let change_plan = ChangePlanner::plan(input.context.objective.clone(), &repo);
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::PlanPrepared,
            format!(
                "change plan prepared with {} step(s)",
                change_plan.steps.len()
            ),
            json!({"step_count": change_plan.steps.len()}),
        ));

        let mut tracker = TurnDiffTracker::new(input.context.workspace_root.clone());
        let mut patch_apply_report = None;
        let mut patch_failure = None;
        let mut all_tests = Vec::new();
        let mut latest_review = CodeReviewAssistant::review("", None);
        let mut latest_iteration =
            PatchIterationPlanner::plan(&input.context.objective, &latest_review, &repo);
        let mut loop_status = CodingLoopStatus::AwaitingFollowUpPatch;
        let mut loop_reason = "mock model did not provide a passing patch".to_owned();
        let mut iterations = 0usize;

        for (index, model_turn) in input.turns.into_iter().take(max_iterations).enumerate() {
            let iteration_number = index + 1;
            iterations = iteration_number;
            events.push(CodingTurnEvent::new(
                CodingTurnEventKind::LoopIterationStarted,
                format!("mock-model coding loop iteration {iteration_number} started"),
                json!({
                    "iteration": iteration_number,
                    "max_iterations": max_iterations,
                    "mode": input.context.mode,
                }),
            ));

            match model_turn.candidate_diff.as_deref() {
                Some(diff) if !diff.trim().is_empty() => {
                    match GuardedPatchApplier::apply_unified_diff(
                        &input.context.workspace_root,
                        diff,
                    ) {
                        Ok(report) => {
                            tracker.track_patch_report(&report)?;
                            events.push(CodingTurnEvent::new(
                                CodingTurnEventKind::PatchApplied,
                                format!(
                                    "patch applied to {} file(s) in iteration {iteration_number}",
                                    report.files_changed
                                ),
                                json!({
                                    "iteration": iteration_number,
                                    "files_changed": report.files_changed,
                                    "files_created": report.files_created,
                                    "files_deleted": report.files_deleted,
                                    "files_moved": report.files_moved,
                                    "hunks_applied": report.hunks_applied,
                                }),
                            ));
                            if let Some(unified_diff) = tracker.unified_diff() {
                                events.push(CodingTurnEvent::new(
                                    CodingTurnEventKind::DiffUpdated,
                                    format!("turn diff updated after iteration {iteration_number}"),
                                    json!({
                                        "iteration": iteration_number,
                                        "summary": tracker.summary(),
                                        "unified_diff": unified_diff,
                                    }),
                                ));
                            }
                            patch_apply_report = Some(report);
                        }
                        Err(error) => {
                            let failure = PatchFailure::from_error(error);
                            events.push(CodingTurnEvent::new(
                                CodingTurnEventKind::PatchFailed,
                                format!(
                                    "patch failed in iteration {iteration_number}: {:?}",
                                    failure.kind
                                ),
                                json!({
                                    "iteration": iteration_number,
                                    "kind": failure.kind,
                                    "path": failure.path,
                                    "message": failure.message,
                                }),
                            ));
                            loop_status = CodingLoopStatus::PatchFailed;
                            loop_reason = format!(
                                "patch failed in iteration {iteration_number} with {:?}",
                                failure.kind
                            );
                            patch_failure = Some(failure);
                            break;
                        }
                    }
                }
                _ => events.push(CodingTurnEvent::new(
                    CodingTurnEventKind::PatchSkipped,
                    format!("mock model did not provide a patch in iteration {iteration_number}"),
                    json!({
                        "iteration": iteration_number,
                        "mode": input.context.mode,
                    }),
                )),
            }

            for test_analysis in &model_turn.test_matrix {
                events.push(CodingTurnEvent::new(
                    CodingTurnEventKind::TestEvidenceRecorded,
                    test_analysis.summary.clone(),
                    json!({
                        "iteration": iteration_number,
                        "command": test_analysis.command,
                        "status": test_analysis.status,
                        "category": test_analysis.category,
                        "failed_tests": test_analysis.failed_tests,
                    }),
                ));
            }
            let primary_test = primary_test_analysis(&model_turn.test_matrix);
            let current_iteration_failed = model_turn
                .test_matrix
                .iter()
                .any(|analysis| analysis.status != 0);
            all_tests.extend(model_turn.test_matrix);
            let review_diff = tracker.unified_diff().unwrap_or_default();
            events.push(CodingTurnEvent::new(
                CodingTurnEventKind::ReviewStarted,
                format!("coding review started for iteration {iteration_number}"),
                json!({
                    "iteration": iteration_number,
                    "diff_chars": review_diff.chars().count(),
                    "has_test_analysis": primary_test.is_some(),
                }),
            ));
            latest_review = CodeReviewAssistant::review(&review_diff, primary_test);
            for finding in &latest_review.findings {
                events.push(CodingTurnEvent::new(
                    CodingTurnEventKind::ReviewFinding,
                    finding.title.clone(),
                    json!({
                        "iteration": iteration_number,
                        "severity": finding.severity.clone(),
                        "title": finding.title.clone(),
                        "detail": finding.detail.clone(),
                        "recommendation": finding.recommendation.clone(),
                        "file": finding.file.clone(),
                    }),
                ));
            }
            events.push(CodingTurnEvent::new(
                CodingTurnEventKind::ReviewCompleted,
                latest_review.summary.clone(),
                json!({
                    "iteration": iteration_number,
                    "finding_count": latest_review.findings.len(),
                    "changed_files": latest_review.changed_files,
                }),
            ));

            latest_iteration =
                PatchIterationPlanner::plan(&input.context.objective, &latest_review, &repo);
            events.push(CodingTurnEvent::new(
                CodingTurnEventKind::IterationPlanned,
                latest_iteration.guarded_edit_objective.clone(),
                json!({
                    "iteration": iteration_number,
                    "priority": latest_iteration.priority,
                    "requires_guarded_edit": latest_iteration.requires_guarded_edit,
                    "ready_for_approval": latest_iteration.ready_for_approval,
                }),
            ));

            if current_iteration_failed {
                loop_status = CodingLoopStatus::AwaitingFollowUpPatch;
                loop_reason = "test evidence still has failures; awaiting follow-up patch".into();
            } else if patch_apply_report.is_some() {
                loop_status = CodingLoopStatus::Passed;
                loop_reason = format!(
                    "mock-model patch/test loop passed after {iteration_number} iteration(s)"
                );
                break;
            }
        }

        let loop_report = CodingLoopReport {
            status: loop_status,
            iterations,
            max_iterations,
            reason: loop_reason,
        };
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::LoopTerminated,
            loop_report.reason.clone(),
            json!({
                "status": loop_report.status,
                "iterations": loop_report.iterations,
                "max_iterations": loop_report.max_iterations,
                "reason": loop_report.reason,
            }),
        ));
        let turn_diff = CodingTurnDiffReport {
            summary: tracker.summary(),
            unified_diff: tracker.unified_diff(),
        };
        let suggested_tests = if input.context.test_commands.is_empty() {
            TestRunnerPlan::infer(&repo)
        } else {
            input.context.test_commands.clone()
        };
        let final_report = render_coding_turn_report(CodingTurnReportView {
            context: &input.context,
            change_plan: &change_plan,
            patch_apply_report: patch_apply_report.as_ref(),
            patch_failure: patch_failure.as_ref(),
            turn_diff: &turn_diff,
            review: &latest_review,
            iteration: &latest_iteration,
            loop_report: &loop_report,
            suggested_tests: &suggested_tests,
        });
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::FinalReportPrepared,
            "mock-model coding turn report prepared",
            json!({
                "event_count": events.len() + 1,
                "has_patch": patch_apply_report.is_some(),
                "has_patch_failure": patch_failure.is_some(),
                "has_turn_diff": turn_diff.unified_diff.is_some(),
            }),
        ));
        let primary_test = primary_test_analysis(&all_tests);
        Ok(CodingTurnReport {
            context: input.context,
            repo,
            change_plan,
            patch_apply_report,
            patch_failure,
            turn_diff,
            suggested_tests,
            test_matrix: all_tests,
            test_analysis: primary_test,
            review: latest_review,
            iteration: latest_iteration,
            loop_report,
            events,
            final_report,
        })
    }
}

impl CodingTurnEvent {
    pub fn new(
        kind: CodingTurnEventKind,
        summary: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            kind,
            summary: redact_secrets(&summary.into()),
            payload: redact_json(payload),
        }
    }
}

fn should_apply_candidate_patch(input: &CodingTurnInput) -> bool {
    input.apply_patch
        && input.candidate_diff.is_some()
        && matches!(
            input.context.mode,
            CodingMode::Edit | CodingMode::SelfModify
        )
}

struct PreparedCodingTurn {
    input: CodingTurnInput,
    events: Vec<CodingTurnEvent>,
    repo: RepoMap,
    change_plan: ChangePlan,
    tracker: TurnDiffTracker,
    patch_apply_report: Option<PatchApplyReport>,
    patch_failure: Option<PatchFailure>,
    review_diff: String,
}

impl PreparedCodingTurn {
    fn new(input: CodingTurnInput) -> Result<Self> {
        let mut events = Vec::new();
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::ContextPrepared,
            format!("coding context prepared for {:?} mode", input.context.mode),
            json!({
                "workspace_root": input.context.workspace_root,
                "session_id": input.context.session_id,
                "turn_id": input.context.turn_id,
                "git": input.context.git,
            }),
        ));
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::GitBaselineCaptured,
            "git baseline captured for coding turn",
            json!({
                "git_root": input.context.git.git_root,
                "head": input.context.git.head,
                "branch": input.context.git.branch,
                "detached": input.context.git.detached,
                "dirty": input.context.git.dirty,
                "has_staged_changes": input.context.git.has_staged_changes,
                "has_unstaged_changes": input.context.git.has_unstaged_changes,
                "has_untracked_files": input.context.git.has_untracked_files,
            }),
        ));
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::LoopIterationStarted,
            "coding loop iteration 1 started",
            json!({
                "iteration": 1,
                "max_iterations": 1,
                "mode": input.context.mode,
            }),
        ));

        let repo = RepoScanner::new(&input.context.workspace_root).scan()?;
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::RepoScanned,
            format!(
                "repo scanned: {} file(s), {} package file(s)",
                repo.files.len(),
                repo.package_files.len()
            ),
            json!({
                "files": repo.files.len(),
                "package_files": repo.package_files.len(),
            }),
        ));

        let change_plan = ChangePlanner::plan(input.context.objective.clone(), &repo);
        events.push(CodingTurnEvent::new(
            CodingTurnEventKind::PlanPrepared,
            format!(
                "change plan prepared with {} step(s)",
                change_plan.steps.len()
            ),
            json!({"step_count": change_plan.steps.len()}),
        ));

        let review_diff = input
            .candidate_diff
            .as_deref()
            .map(redact_secrets)
            .unwrap_or_default();
        let workspace_root = input.context.workspace_root.clone();

        Ok(Self {
            input,
            events,
            repo,
            change_plan,
            tracker: TurnDiffTracker::new(workspace_root),
            patch_apply_report: None,
            patch_failure: None,
            review_diff,
        })
    }

    fn record_patch_applied(&mut self, report: PatchApplyReport) -> Result<()> {
        self.tracker.track_patch_report(&report)?;
        self.events.push(CodingTurnEvent::new(
            CodingTurnEventKind::PatchApplied,
            format!("patch applied to {} file(s)", report.files_changed),
            json!({
                "files_changed": report.files_changed,
                "files_created": report.files_created,
                "files_deleted": report.files_deleted,
                "files_moved": report.files_moved,
                "hunks_applied": report.hunks_applied,
            }),
        ));
        if let Some(unified_diff) = self.tracker.unified_diff() {
            self.review_diff = unified_diff.clone();
            self.events.push(CodingTurnEvent::new(
                CodingTurnEventKind::DiffUpdated,
                "turn diff updated after patch apply",
                json!({
                    "summary": self.tracker.summary(),
                    "unified_diff": unified_diff,
                }),
            ));
        }
        self.patch_apply_report = Some(report);
        Ok(())
    }

    fn record_patch_failed(&mut self, failure: PatchFailure) {
        self.events.push(CodingTurnEvent::new(
            CodingTurnEventKind::PatchFailed,
            format!("patch failed: {:?}", failure.kind),
            json!({
                "kind": failure.kind,
                "path": failure.path,
                "message": failure.message,
            }),
        ));
        self.patch_failure = Some(failure);
    }

    fn record_patch_skipped(&mut self) {
        self.events.push(CodingTurnEvent::new(
            CodingTurnEventKind::PatchSkipped,
            patch_skip_summary(&self.input),
            json!({
                "mode": self.input.context.mode,
                "apply_patch": self.input.apply_patch,
                "has_candidate_diff": self.input.candidate_diff.is_some(),
            }),
        ));
    }

    fn finish(mut self) -> CodingTurnReport {
        let test_matrix = normalized_test_matrix(&self.input);
        for test_analysis in &test_matrix {
            self.events.push(CodingTurnEvent::new(
                CodingTurnEventKind::TestEvidenceRecorded,
                test_analysis.summary.clone(),
                json!({
                    "command": test_analysis.command,
                    "status": test_analysis.status,
                    "category": test_analysis.category,
                    "failed_tests": test_analysis.failed_tests,
                }),
            ));
        }
        let primary_test_analysis = primary_test_analysis(&test_matrix);

        let suggested_tests = if self.input.context.test_commands.is_empty() {
            TestRunnerPlan::infer(&self.repo)
        } else {
            self.input.context.test_commands.clone()
        };
        self.events.push(CodingTurnEvent::new(
            CodingTurnEventKind::ReviewStarted,
            "coding review started",
            json!({
                "diff_chars": self.review_diff.chars().count(),
                "has_test_analysis": primary_test_analysis.is_some(),
                "test_result_count": test_matrix.len(),
            }),
        ));
        let review = CodeReviewAssistant::review(&self.review_diff, primary_test_analysis.clone());
        for finding in &review.findings {
            self.events.push(CodingTurnEvent::new(
                CodingTurnEventKind::ReviewFinding,
                finding.title.clone(),
                json!({
                    "severity": finding.severity.clone(),
                    "title": finding.title.clone(),
                    "detail": finding.detail.clone(),
                    "recommendation": finding.recommendation.clone(),
                    "file": finding.file.clone(),
                }),
            ));
        }
        self.events.push(CodingTurnEvent::new(
            CodingTurnEventKind::ReviewCompleted,
            review.summary.clone(),
            json!({
                "finding_count": review.findings.len(),
                "changed_files": review.changed_files,
            }),
        ));

        let iteration =
            PatchIterationPlanner::plan(&self.input.context.objective, &review, &self.repo);
        self.events.push(CodingTurnEvent::new(
            CodingTurnEventKind::IterationPlanned,
            iteration.guarded_edit_objective.clone(),
            json!({
                "priority": iteration.priority,
                "requires_guarded_edit": iteration.requires_guarded_edit,
                "ready_for_approval": iteration.ready_for_approval,
            }),
        ));
        let loop_report = build_loop_report(&self, &test_matrix, &review);
        self.events.push(CodingTurnEvent::new(
            CodingTurnEventKind::LoopTerminated,
            loop_report.reason.clone(),
            json!({
                "status": loop_report.status,
                "iterations": loop_report.iterations,
                "max_iterations": loop_report.max_iterations,
                "reason": loop_report.reason,
            }),
        ));

        let turn_diff = CodingTurnDiffReport {
            summary: self.tracker.summary(),
            unified_diff: self.tracker.unified_diff(),
        };
        let final_report = render_coding_turn_report(CodingTurnReportView {
            context: &self.input.context,
            change_plan: &self.change_plan,
            patch_apply_report: self.patch_apply_report.as_ref(),
            patch_failure: self.patch_failure.as_ref(),
            turn_diff: &turn_diff,
            review: &review,
            iteration: &iteration,
            loop_report: &loop_report,
            suggested_tests: &suggested_tests,
        });
        self.events.push(CodingTurnEvent::new(
            CodingTurnEventKind::FinalReportPrepared,
            "coding turn report prepared",
            json!({
                "event_count": self.events.len() + 1,
                "has_patch": self.patch_apply_report.is_some(),
                "has_patch_failure": self.patch_failure.is_some(),
                "has_turn_diff": turn_diff.unified_diff.is_some(),
            }),
        ));

        CodingTurnReport {
            context: self.input.context,
            repo: self.repo,
            change_plan: self.change_plan,
            patch_apply_report: self.patch_apply_report,
            patch_failure: self.patch_failure,
            turn_diff,
            suggested_tests,
            test_matrix,
            test_analysis: primary_test_analysis,
            review,
            iteration,
            loop_report,
            events: self.events,
            final_report,
        }
    }
}

fn patch_skip_summary(input: &CodingTurnInput) -> String {
    if input.candidate_diff.is_none() {
        return "no candidate diff supplied".into();
    }
    if !input.apply_patch {
        return "candidate patch retained for review without apply".into();
    }
    format!(
        "candidate patch not applied in {:?} coding mode",
        input.context.mode
    )
}

fn build_loop_report(
    turn: &PreparedCodingTurn,
    test_matrix: &[TestFailureAnalysis],
    review: &ReviewReport,
) -> CodingLoopReport {
    let (status, reason) = if let Some(failure) = &turn.patch_failure {
        (
            CodingLoopStatus::PatchFailed,
            format!("patch failed with {:?}", failure.kind),
        )
    } else if test_matrix.iter().any(|analysis| analysis.status != 0) {
        (
            CodingLoopStatus::AwaitingFollowUpPatch,
            "test evidence still has failures; awaiting follow-up patch".into(),
        )
    } else if !test_matrix.is_empty() && turn.patch_apply_report.is_some() {
        (
            CodingLoopStatus::Passed,
            "patch applied and test evidence passed".into(),
        )
    } else if review.findings.is_empty() {
        (
            CodingLoopStatus::Passed,
            "review found no blockers in deterministic coding turn".into(),
        )
    } else {
        (
            CodingLoopStatus::ReviewBlocked,
            "review findings require a follow-up patch".into(),
        )
    };
    CodingLoopReport {
        status,
        iterations: 1,
        max_iterations: 1,
        reason,
    }
}

fn normalized_test_matrix(input: &CodingTurnInput) -> Vec<TestFailureAnalysis> {
    if !input.test_matrix.is_empty() {
        return input.test_matrix.clone();
    }
    input.test_analysis.clone().into_iter().collect()
}

fn primary_test_analysis(test_matrix: &[TestFailureAnalysis]) -> Option<TestFailureAnalysis> {
    test_matrix
        .iter()
        .find(|analysis| analysis.status != 0)
        .or_else(|| test_matrix.first())
        .cloned()
}

struct CodingTurnReportView<'a> {
    context: &'a CodingTurnContext,
    change_plan: &'a ChangePlan,
    patch_apply_report: Option<&'a PatchApplyReport>,
    patch_failure: Option<&'a PatchFailure>,
    turn_diff: &'a CodingTurnDiffReport,
    review: &'a ReviewReport,
    iteration: &'a PatchIterationPlan,
    loop_report: &'a CodingLoopReport,
    suggested_tests: &'a [TestCommand],
}

fn render_coding_turn_report(input: CodingTurnReportView<'_>) -> String {
    let mut report = String::new();
    report.push_str("# Coding Turn Report\n\n");
    report.push_str(&format!(
        "Objective: {}\n\n",
        redact_secrets(&input.context.objective)
    ));
    report.push_str(&format!("Mode: {:?}\n\n", input.context.mode));
    report.push_str("## Plan\n\n");
    for step in &input.change_plan.steps {
        report.push_str(&format!("- {}\n", redact_secrets(step)));
    }
    report.push_str("\n## Patch\n\n");
    match input.patch_apply_report {
        Some(patch) => report.push_str(&format!(
            "- Applied: {} file(s), {} hunk(s), {} insertion(s), {} deletion(s)\n",
            patch.files_changed, patch.hunks_applied, patch.insertions, patch.deletions
        )),
        None => report.push_str("- Applied: false\n"),
    }
    if let Some(failure) = input.patch_failure {
        report.push_str(&format!(
            "- Failure: {:?}: {}\n",
            failure.kind,
            redact_secrets(&failure.message)
        ));
    }
    report.push_str(&format!(
        "- Turn diff available: {}\n",
        input.turn_diff.unified_diff.is_some()
    ));
    report.push_str("\n## Tests\n\n");
    if input.suggested_tests.is_empty() {
        report.push_str("- none inferred\n");
    } else {
        for command in input.suggested_tests {
            report.push_str(&format!(
                "- `{}`: {}\n",
                redact_secrets(&command.command),
                redact_secrets(&command.reason)
            ));
        }
    }
    report.push_str("\n## Review\n\n");
    report.push_str(&format!("- {}\n", redact_secrets(&input.review.summary)));
    report.push_str("\n## Next Iteration\n\n");
    report.push_str(&format!(
        "- Requires guarded edit: {}\n",
        input.iteration.requires_guarded_edit
    ));
    report.push_str(&format!(
        "- Ready for approval: {}\n",
        input.iteration.ready_for_approval
    ));
    report.push_str("\n## Loop\n\n");
    report.push_str(&format!("- Status: {:?}\n", input.loop_report.status));
    report.push_str(&format!(
        "- Reason: {}\n",
        redact_secrets(&input.loop_report.reason)
    ));
    report
}
