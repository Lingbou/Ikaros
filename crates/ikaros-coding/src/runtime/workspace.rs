// SPDX-License-Identifier: GPL-3.0-only

use super::{
    CodingTurnDiffReport, CodingTurnEvent, CodingTurnEventKind, CodingTurnInput, CodingTurnReport,
    check::{
        build_loop_report, normalized_test_matrix, patch_skip_summary, primary_test_analysis,
        should_apply_candidate_patch,
    },
    report::{CodingTurnReportView, render_coding_turn_report},
};
use crate::{
    ChangePlan, ChangePlanner, CodeReviewAssistant, GuardedPatchApplier, PatchApplyReport,
    PatchFailure, PatchIterationPlanner, RepoMap, RepoScanner, TestRunnerPlan, TurnDiffTracker,
};
use ikaros_core::{Result, redact_secrets};
use ikaros_sandbox::FileSystem as ExecutionFileSystem;
use serde_json::json;

pub(super) async fn run_turn_with_env(
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
        let loop_report = build_loop_report(
            self.patch_failure.as_ref(),
            self.patch_apply_report.is_some(),
            &test_matrix,
            &review,
        );
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
