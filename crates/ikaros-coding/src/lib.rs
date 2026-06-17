// SPDX-License-Identifier: GPL-3.0-only
//! Coding capability primitives for Ikaros.

mod iteration;
mod patch;
mod repo;
mod review;
mod self_modify;
mod testing;
mod workflow;

pub use iteration::{PatchIterationPlan, PatchIterationPlanner};
pub use patch::{GuardedPatchApplier, PatchApplyReport};
pub use repo::{
    ChangePlan, ChangePlanner, RepoFile, RepoFileKind, RepoMap, RepoScanner, TestCommand,
    TestRunnerPlan,
};
pub use review::{
    CodeReviewAssistant, DiffSummarizer, DiffSummary, ReviewFinding, ReviewReport, ReviewSeverity,
};
pub use self_modify::{
    SelfModifyApplyReport, SelfModifyChangeKind, SelfModifyCheckProfile, SelfModifyCheckReport,
    SelfModifyDryRunReport, SelfModifyHeartbeatReport, SelfModifyOperationKind,
    SelfModifyOperationRecord, SelfModifyProposal, SelfModifyRollbackPlan,
    SelfModifyRollbackReport, SelfModifyStore,
};
pub use testing::{
    TestFailureAnalysis, TestFailureAnalyzer, TestFailureCategory, is_allowed_test_command,
    validate_test_command,
};
pub use workflow::{
    CodingWorkflow, CodingWorkflowInput, CodingWorkflowReport, CodingWorkflowStep,
    CodingWorkflowStepKind, CodingWorkflowStepStatus,
};

#[cfg(test)]
mod tests;
