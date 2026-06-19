// SPDX-License-Identifier: GPL-3.0-only
//! Coding capability primitives for Ikaros.

mod context;
mod diff;
mod iteration;
mod patch;
mod repo;
mod review;
mod runtime;
mod self_modify;
mod testing;
mod workflow;

pub use context::{
    CodingDirtyState, CodingGitState, CodingMode, CodingModeCapabilities, CodingPermissionProfile,
    CodingTurnContext, CodingTurnContextInput,
};
pub use diff::{
    TurnDiffFile, TurnDiffFileStatus, TurnDiffSummary, TurnDiffTracker, UnifiedDiffRender,
};
pub use iteration::{PatchIterationPlan, PatchIterationPlanner};
pub use patch::{
    GuardedPatchApplier, PatchApplyReport, PatchFailure, PatchFailureKind, PatchFileChange,
    PatchFileOperation,
};
pub use repo::{
    ChangePlan, ChangePlanner, RepoFile, RepoFileKind, RepoMap, RepoScanner, TestCommand,
    TestRunnerPlan,
};
pub use review::{
    CodeReviewAssistant, DiffSummarizer, DiffSummary, ReviewFinding, ReviewReport, ReviewSeverity,
};
pub use runtime::{
    CodingLoopReport, CodingLoopStatus, CodingRuntime, CodingTurnDiffReport, CodingTurnEvent,
    CodingTurnEventKind, CodingTurnInput, CodingTurnReport, DeterministicCodingRuntime,
    MockModelCodingInput, MockModelCodingRuntime, MockModelCodingTurn,
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
