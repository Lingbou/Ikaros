// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    ChangePlan, ChangePlanner, CodeReviewAssistant, DiffSummarizer, DiffSummary,
    PatchIterationPlan, PatchIterationPlanner, RepoMap, RepoScanner, TestCommand,
    TestFailureAnalysis, TestRunnerPlan,
};
use ikaros_core::{Result, redact_secrets};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingWorkflowInput {
    pub objective: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_analysis: Option<TestFailureAnalysis>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingWorkflowReport {
    pub objective: String,
    pub repo: RepoMap,
    pub change_plan: ChangePlan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff_summary: Option<DiffSummary>,
    pub suggested_tests: Vec<TestCommand>,
    pub review: crate::ReviewReport,
    pub iteration: PatchIterationPlan,
    pub steps: Vec<CodingWorkflowStep>,
    pub requires_guarded_edit: bool,
    pub ready_for_approval: bool,
    pub final_report: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodingWorkflowStepKind {
    ReadRepo,
    Plan,
    Patch,
    Test,
    Review,
    FinalReport,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CodingWorkflowStepStatus {
    Planned,
    Completed,
    Blocked,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodingWorkflowStep {
    pub kind: CodingWorkflowStepKind,
    pub status: CodingWorkflowStepStatus,
    pub summary: String,
}

pub struct CodingWorkflow {
    root: PathBuf,
}

impl CodingWorkflow {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn run(&self, input: CodingWorkflowInput) -> Result<CodingWorkflowReport> {
        let objective = redact_secrets(&input.objective);
        let diff = input.diff.map(|diff| redact_secrets(&diff));
        let repo = RepoScanner::new(&self.root).scan()?;
        let mut steps = Vec::new();
        steps.push(CodingWorkflowStep::completed(
            CodingWorkflowStepKind::ReadRepo,
            format!(
                "repo scanned: {} file(s), {} package file(s)",
                repo.files.len(),
                repo.package_files.len()
            ),
        ));

        let change_plan = ChangePlanner::plan(objective.clone(), &repo);
        steps.push(CodingWorkflowStep::completed(
            CodingWorkflowStepKind::Plan,
            format!(
                "change plan prepared with {} step(s)",
                change_plan.steps.len()
            ),
        ));

        let diff_summary = diff
            .as_deref()
            .filter(|diff| !diff.trim().is_empty())
            .map(DiffSummarizer::summarize);
        steps.push(match &diff_summary {
            Some(summary) => CodingWorkflowStep::completed(
                CodingWorkflowStepKind::Patch,
                format!(
                    "candidate patch captured for review: {}",
                    redact_secrets(&summary.summary)
                ),
            ),
            None => CodingWorkflowStep::blocked(
                CodingWorkflowStepKind::Patch,
                "no candidate diff supplied; guarded edit must produce a unified diff first",
            ),
        });

        let suggested_tests = TestRunnerPlan::infer(&repo);
        steps.push(match &input.test_analysis {
            Some(analysis) => CodingWorkflowStep::completed(
                CodingWorkflowStepKind::Test,
                format!("test evidence recorded: {}", analysis.summary),
            ),
            None => CodingWorkflowStep::planned(
                CodingWorkflowStepKind::Test,
                format!(
                    "{} test command(s) inferred; no test evidence recorded yet",
                    suggested_tests.len()
                ),
            ),
        });

        let review = CodeReviewAssistant::review(
            diff.as_deref().unwrap_or_default(),
            input.test_analysis.clone(),
        );
        steps.push(CodingWorkflowStep::completed(
            CodingWorkflowStepKind::Review,
            review.summary.clone(),
        ));

        let iteration = PatchIterationPlanner::plan(objective.clone(), &review, &repo);
        let requires_guarded_edit = iteration.requires_guarded_edit;
        let ready_for_approval = iteration.ready_for_approval;
        let final_report = render_final_report(
            &objective,
            &change_plan,
            &review,
            &iteration,
            requires_guarded_edit,
            ready_for_approval,
        );
        steps.push(CodingWorkflowStep::completed(
            CodingWorkflowStepKind::FinalReport,
            "final coding workflow report prepared",
        ));

        Ok(CodingWorkflowReport {
            objective,
            repo,
            change_plan,
            diff_summary,
            suggested_tests,
            review,
            iteration,
            steps,
            requires_guarded_edit,
            ready_for_approval,
            final_report,
        })
    }
}

impl CodingWorkflowStep {
    fn planned(kind: CodingWorkflowStepKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            status: CodingWorkflowStepStatus::Planned,
            summary: redact_secrets(&summary.into()),
        }
    }

    fn completed(kind: CodingWorkflowStepKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            status: CodingWorkflowStepStatus::Completed,
            summary: redact_secrets(&summary.into()),
        }
    }

    fn blocked(kind: CodingWorkflowStepKind, summary: impl Into<String>) -> Self {
        Self {
            kind,
            status: CodingWorkflowStepStatus::Blocked,
            summary: redact_secrets(&summary.into()),
        }
    }
}

fn render_final_report(
    objective: &str,
    change_plan: &ChangePlan,
    review: &crate::ReviewReport,
    iteration: &PatchIterationPlan,
    requires_guarded_edit: bool,
    ready_for_approval: bool,
) -> String {
    let mut report = String::new();
    report.push_str("# Final Report\n\n");
    report.push_str(&format!("Objective: {}\n\n", redact_secrets(objective)));
    report.push_str("## Workflow\n\n");
    for step in &change_plan.steps {
        report.push_str(&format!("- {}\n", redact_secrets(step)));
    }
    report.push_str("\n## Review\n\n");
    report.push_str(&format!("- {}\n", redact_secrets(&review.summary)));
    report.push_str("\n\n## Next Patch\n\n");
    report.push_str(&format!(
        "- Requires guarded edit: {}\n",
        requires_guarded_edit
    ));
    report.push_str(&format!("- Ready for approval: {}\n", ready_for_approval));
    report.push_str(&format!(
        "- Guarded edit objective: {}\n",
        redact_secrets(&iteration.guarded_edit_objective)
    ));
    report
}
