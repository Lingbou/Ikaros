// SPDX-License-Identifier: GPL-3.0-only

use super::{CodingLoopReport, CodingTurnDiffReport};
use crate::{
    ChangePlan, CodingTurnContext, PatchApplyReport, PatchFailure, PatchIterationPlan,
    ReviewReport, TestCommand,
};
use ikaros_core::redact_secrets;

pub(super) struct CodingTurnReportView<'a> {
    pub(super) context: &'a CodingTurnContext,
    pub(super) change_plan: &'a ChangePlan,
    pub(super) patch_apply_report: Option<&'a PatchApplyReport>,
    pub(super) patch_failure: Option<&'a PatchFailure>,
    pub(super) turn_diff: &'a CodingTurnDiffReport,
    pub(super) review: &'a ReviewReport,
    pub(super) iteration: &'a PatchIterationPlan,
    pub(super) loop_report: &'a CodingLoopReport,
    pub(super) suggested_tests: &'a [TestCommand],
}

pub(super) fn render_coding_turn_report(input: CodingTurnReportView<'_>) -> String {
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
