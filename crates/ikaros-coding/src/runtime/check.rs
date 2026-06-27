// SPDX-License-Identifier: GPL-3.0-only

use super::{CodingLoopReport, CodingLoopStatus, CodingTurnInput};
use crate::{CodingMode, PatchFailure, ReviewReport, TestFailureAnalysis};

pub(super) fn should_apply_candidate_patch(input: &CodingTurnInput) -> bool {
    input.apply_patch
        && input.candidate_diff.is_some()
        && matches!(
            input.context.mode,
            CodingMode::Edit | CodingMode::SelfModify
        )
}

pub(super) fn patch_skip_summary(input: &CodingTurnInput) -> String {
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

pub(super) fn normalized_test_matrix(input: &CodingTurnInput) -> Vec<TestFailureAnalysis> {
    if !input.test_matrix.is_empty() {
        return input.test_matrix.clone();
    }
    input.test_analysis.clone().into_iter().collect()
}

pub(super) fn primary_test_analysis(
    test_matrix: &[TestFailureAnalysis],
) -> Option<TestFailureAnalysis> {
    test_matrix
        .iter()
        .find(|analysis| analysis.status != 0)
        .or_else(|| test_matrix.first())
        .cloned()
}

pub(super) fn build_loop_report(
    patch_failure: Option<&PatchFailure>,
    patch_applied: bool,
    test_matrix: &[TestFailureAnalysis],
    review: &ReviewReport,
) -> CodingLoopReport {
    let (status, reason) = if let Some(failure) = patch_failure {
        (
            CodingLoopStatus::PatchFailed,
            format!("patch failed with {:?}", failure.kind),
        )
    } else if test_matrix.iter().any(|analysis| analysis.status != 0) {
        (
            CodingLoopStatus::AwaitingFollowUpPatch,
            "test evidence still has failures; awaiting follow-up patch".into(),
        )
    } else if !test_matrix.is_empty() && patch_applied {
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
