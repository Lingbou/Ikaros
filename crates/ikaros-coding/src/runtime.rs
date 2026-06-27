// SPDX-License-Identifier: GPL-3.0-only

mod check;
mod command;
mod report;
mod workspace;

use crate::{
    CodingTurnContext, PatchApplyReport, PatchFailure, PatchIterationPlan, RepoMap, ReviewReport,
    TestCommand, TestFailureAnalysis, TurnDiffSummary,
};
use ikaros_core::{Result, redact_json, redact_secrets};
use ikaros_sandbox::FileSystem as ExecutionFileSystem;
use serde::{Deserialize, Serialize};

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
    pub change_plan: crate::ChangePlan,
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

impl DeterministicCodingRuntime {
    pub async fn run_turn_with_env(
        &self,
        input: CodingTurnInput,
        file_system: &dyn ExecutionFileSystem,
    ) -> Result<CodingTurnReport> {
        workspace::run_turn_with_env(input, file_system).await
    }
}

impl MockModelCodingRuntime {
    pub async fn run_scripted_turns_with_env(
        &self,
        input: MockModelCodingInput,
        file_system: &dyn ExecutionFileSystem,
    ) -> Result<CodingTurnReport> {
        command::run_scripted_turns_with_env(self.max_iterations, input, file_system).await
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
