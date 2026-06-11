// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::TaskState;
use ikaros_harness::{PlanStepStatus, TaskExecutionReport};
use ikaros_soul::RuntimeSignal;

pub fn task_report_succeeded(report: &TaskExecutionReport) -> bool {
    report
        .steps
        .iter()
        .all(|step| step.status == PlanStepStatus::Succeeded)
}

pub fn task_report_summary(
    report: &TaskExecutionReport,
    completed_summary: impl Into<String>,
) -> String {
    if task_report_succeeded(report) {
        return completed_summary.into();
    }
    report
        .steps
        .iter()
        .find(|step| step.status != PlanStepStatus::Succeeded)
        .map(|step| format!("{:?}: {}", step.status, step.summary))
        .unwrap_or_else(|| format!("{:?}", report.state))
}

pub(super) fn task_emotion_signal(report: &TaskExecutionReport) -> RuntimeSignal {
    if report
        .steps
        .iter()
        .any(|step| step.status == PlanStepStatus::WaitingForApproval)
    {
        return RuntimeSignal::RiskAction;
    }
    match report.state {
        TaskState::Completed => RuntimeSignal::TaskComplete,
        TaskState::Failed => RuntimeSignal::TestFailure,
        TaskState::WaitingForApproval | TaskState::Blocked | TaskState::Cancelled => {
            RuntimeSignal::RiskAction
        }
        TaskState::Created | TaskState::Planning | TaskState::Running => RuntimeSignal::Planning,
    }
}

pub(super) fn task_emotion_reason(report: &TaskExecutionReport) -> &'static str {
    match task_emotion_signal(report) {
        RuntimeSignal::TaskComplete => "task completed",
        RuntimeSignal::TestFailure => "task failed",
        RuntimeSignal::RiskAction => "task requires attention",
        RuntimeSignal::Planning => "task still in progress",
        RuntimeSignal::Research => "task researched local context",
        RuntimeSignal::Idle => "task idle",
    }
}
