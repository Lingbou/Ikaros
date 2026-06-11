// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    repo::{RepoMap, TestCommand, TestRunnerPlan},
    review::{DiffSummary, ReviewFinding, ReviewReport, ReviewSeverity},
    testing::TestFailureCategory,
};
use ikaros_core::redact_secrets;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PatchIterationPlan {
    pub objective: String,
    pub priority: ReviewSeverity,
    pub diff_summary: DiffSummary,
    pub changed_files: Vec<PathBuf>,
    pub blockers: Vec<ReviewFinding>,
    pub iteration_steps: Vec<String>,
    pub suggested_tests: Vec<TestCommand>,
    pub guarded_edit_objective: String,
    pub requires_guarded_edit: bool,
    pub ready_for_approval: bool,
    pub markdown: String,
}

pub struct PatchIterationPlanner;

impl PatchIterationPlanner {
    pub fn plan(
        objective: impl Into<String>,
        review: &ReviewReport,
        repo: &RepoMap,
    ) -> PatchIterationPlan {
        let objective = redact_secrets(&objective.into());
        let priority = highest_review_severity(&review.findings);
        let blockers = review_blockers(&review.findings);
        let requires_guarded_edit = review.findings.iter().any(|finding| {
            matches!(
                finding.title.as_str(),
                "Secret-like content added"
                    | "Unsafe code added"
                    | "Placeholder runtime failure added"
                    | "Potential panic path added"
                    | "Debug output added"
                    | "Tests are not passing"
            )
        });
        let ready_for_approval = review.diff_summary.files_changed > 0
            && blockers.is_empty()
            && matches!(
                review
                    .test_analysis
                    .as_ref()
                    .map(|analysis| &analysis.category),
                Some(TestFailureCategory::Passed)
            );
        let iteration_steps = patch_iteration_steps(review, requires_guarded_edit);
        let suggested_tests = TestRunnerPlan::infer(repo);
        let guarded_edit_objective =
            guarded_edit_objective(&objective, review, requires_guarded_edit);
        let markdown = render_patch_iteration_markdown(PatchIterationMarkdownInput {
            objective: &objective,
            priority: &priority,
            review,
            blockers: &blockers,
            steps: &iteration_steps,
            tests: &suggested_tests,
            guarded_edit_objective: &guarded_edit_objective,
            requires_guarded_edit,
            ready_for_approval,
        });
        PatchIterationPlan {
            objective,
            priority,
            diff_summary: review.diff_summary.clone(),
            changed_files: review.changed_files.clone(),
            blockers,
            iteration_steps,
            suggested_tests,
            guarded_edit_objective,
            requires_guarded_edit,
            ready_for_approval,
            markdown,
        }
    }
}

fn highest_review_severity(findings: &[ReviewFinding]) -> ReviewSeverity {
    if findings
        .iter()
        .any(|finding| finding.severity == ReviewSeverity::High)
    {
        ReviewSeverity::High
    } else if findings
        .iter()
        .any(|finding| finding.severity == ReviewSeverity::Medium)
    {
        ReviewSeverity::Medium
    } else if findings
        .iter()
        .any(|finding| finding.severity == ReviewSeverity::Low)
    {
        ReviewSeverity::Low
    } else {
        ReviewSeverity::Info
    }
}

fn review_blockers(findings: &[ReviewFinding]) -> Vec<ReviewFinding> {
    findings
        .iter()
        .filter(|finding| {
            matches!(
                finding.severity,
                ReviewSeverity::High | ReviewSeverity::Medium
            ) || matches!(
                finding.title.as_str(),
                "No test analysis provided" | "Tests are not passing" | "No diff detected"
            )
        })
        .cloned()
        .collect()
}

fn patch_iteration_steps(review: &ReviewReport, requires_guarded_edit: bool) -> Vec<String> {
    let mut steps =
        vec!["Use the current review report as the source of truth for the next iteration.".into()];
    if review.diff_summary.files_changed == 0 {
        steps.push("Collect or create a focused unified diff before planning an edit.".into());
    }
    for finding in &review.findings {
        match finding.title.as_str() {
            "Secret-like content added" => steps.push(
                "Remove secret-like additions and route credentials through configuration or secret adapters.".into(),
            ),
            "Unsafe code added" => steps.push(
                "Isolate unsafe code, document its invariant, or replace it with a safe path."
                    .into(),
            ),
            "Placeholder runtime failure added" => {
                steps.push("Replace todo/unimplemented placeholders with real behavior.".into());
            }
            "Potential panic path added" => steps.push(
                "Replace unwrap/expect/panic paths with error propagation or a documented invariant."
                    .into(),
            ),
            "Debug output added" => {
                steps.push("Remove temporary debug output or confirm it is intentional CLI output.".into());
            }
            "Tests are not passing" => {
                steps.push("Fix failing tests before requesting guarded edit approval.".into());
            }
            "No test analysis provided" => steps.push(
                "Run a focused `ikaros test run --command \"<test command>\"` before approval.".into(),
            ),
            _ => {}
        }
    }
    if requires_guarded_edit {
        steps.push(
            "Prepare the smallest unified diff that addresses the blockers, then submit it through `ikaros code guarded-edit --diff <unified-diff>`."
                .into(),
        );
    }
    steps.push("Regenerate `ikaros code review` after the guarded patch is applied.".into());
    steps.sort();
    steps.dedup();
    steps
}

fn guarded_edit_objective(
    objective: &str,
    review: &ReviewReport,
    requires_guarded_edit: bool,
) -> String {
    if !requires_guarded_edit {
        return redact_secrets(&format!(
            "No guarded edit required for {objective}; verify tests and residual risk."
        ));
    }
    let files = if review.changed_files.is_empty() {
        "reviewed files".into()
    } else {
        review
            .changed_files
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    };
    let finding_titles = review
        .findings
        .iter()
        .filter(|finding| {
            matches!(
                finding.severity,
                ReviewSeverity::High | ReviewSeverity::Medium
            )
        })
        .map(|finding| finding.title.clone())
        .collect::<Vec<_>>();
    let focus = if finding_titles.is_empty() {
        "review findings".into()
    } else {
        finding_titles.join(", ")
    };
    redact_secrets(&format!("{objective}: address {focus} in {files}"))
}

struct PatchIterationMarkdownInput<'a> {
    objective: &'a str,
    priority: &'a ReviewSeverity,
    review: &'a ReviewReport,
    blockers: &'a [ReviewFinding],
    steps: &'a [String],
    tests: &'a [TestCommand],
    guarded_edit_objective: &'a str,
    requires_guarded_edit: bool,
    ready_for_approval: bool,
}

fn render_patch_iteration_markdown(input: PatchIterationMarkdownInput<'_>) -> String {
    let mut markdown = String::new();
    markdown.push_str("# Patch Iteration Plan\n\n");
    markdown.push_str(&format!(
        "Objective: {}\n\n",
        redact_secrets(input.objective)
    ));
    markdown.push_str(&format!("Priority: {:?}\n\n", input.priority));
    markdown.push_str(&format!(
        "Diff: {}\n\n",
        redact_secrets(&input.review.diff_summary.summary)
    ));
    markdown.push_str(&format!(
        "Requires guarded edit: {}\n\nReady for approval: {}\n\n",
        input.requires_guarded_edit, input.ready_for_approval
    ));
    markdown.push_str("## Blockers\n\n");
    if input.blockers.is_empty() {
        markdown.push_str("- none\n");
    } else {
        for finding in input.blockers {
            let file = finding
                .file
                .as_ref()
                .map(|path| format!(" ({})", path.display()))
                .unwrap_or_default();
            markdown.push_str(&format!(
                "- [{:?}] {}{}: {}\n",
                finding.severity,
                redact_secrets(&finding.title),
                file,
                redact_secrets(&finding.recommendation),
            ));
        }
    }
    markdown.push_str("\n## Iteration Steps\n\n");
    for step in input.steps {
        markdown.push_str(&format!("- {}\n", redact_secrets(step)));
    }
    markdown.push_str("\n## Suggested Tests\n\n");
    if input.tests.is_empty() {
        markdown.push_str("- no project-specific test commands inferred\n");
    } else {
        for test in input.tests {
            markdown.push_str(&format!(
                "- `{}`: {}\n",
                redact_secrets(&test.command),
                redact_secrets(&test.reason)
            ));
        }
    }
    markdown.push_str("\n## Guarded Edit Objective\n\n");
    markdown.push_str(&format!(
        "- {}\n",
        redact_secrets(input.guarded_edit_objective)
    ));
    markdown
}
