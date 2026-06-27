// SPDX-License-Identifier: GPL-3.0-only

use super::{
    CodingLoopReport, CodingLoopStatus, CodingTurnDiffReport, CodingTurnEvent, CodingTurnEventKind,
    CodingTurnReport, MockModelCodingInput,
    check::primary_test_analysis,
    report::{CodingTurnReportView, render_coding_turn_report},
};
use crate::{
    ChangePlanner, CodeReviewAssistant, GuardedPatchApplier, PatchIterationPlanner, RepoScanner,
    TestRunnerPlan, TurnDiffTracker,
};
use ikaros_core::Result;
use ikaros_sandbox::FileSystem as ExecutionFileSystem;
use serde_json::json;

pub(super) async fn run_scripted_turns_with_env(
    max_iterations: usize,
    input: MockModelCodingInput,
    file_system: &dyn ExecutionFileSystem,
) -> Result<CodingTurnReport> {
    let max_iterations = max_iterations.max(1).min(input.turns.len().max(1));
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
                match GuardedPatchApplier::apply_unified_diff_with_env_checked(
                    &input.context.workspace_root,
                    diff,
                    file_system,
                )
                .await
                {
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
                    Err(failure) => {
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
            loop_reason =
                format!("mock-model patch/test loop passed after {iteration_number} iteration(s)");
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
