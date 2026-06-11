// SPDX-License-Identifier: GPL-3.0-only

use crate::{DiffSummary, PatchApplyReport, ReviewFinding, TestFailureAnalysis};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelfModifyChangeKind {
    SkillPatch,
    PersonaPatch,
    ConfigPatch,
    RuntimePatch,
    DocumentationPatch,
}

impl SelfModifyChangeKind {
    pub fn as_config_key(&self) -> &'static str {
        match self {
            Self::SkillPatch => "skill_patch",
            Self::PersonaPatch => "persona_patch",
            Self::ConfigPatch => "config_patch",
            Self::RuntimePatch => "runtime_patch",
            Self::DocumentationPatch => "documentation_patch",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfModifyProposal {
    pub id: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposer_task_id: Option<String>,
    pub change_kind: SelfModifyChangeKind,
    pub target_path: PathBuf,
    pub unified_diff: String,
    pub dry_run_report: SelfModifyDryRunReport,
    pub rollback_plan: SelfModifyRollbackPlan,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfModifyDryRunReport {
    pub enabled: bool,
    pub apply_available: bool,
    #[serde(default)]
    pub manual_apply_available: bool,
    pub ok_to_request_approval: bool,
    pub target_path: PathBuf,
    pub diff_summary: DiffSummary,
    pub changed_files: Vec<PathBuf>,
    pub findings: Vec<ReviewFinding>,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfModifyRollbackPlan {
    pub snapshot_required: bool,
    pub snapshot_path: PathBuf,
    pub target_existed: bool,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfModifyHeartbeatReport {
    pub at: String,
    pub status: String,
    pub proposal_count: usize,
    pub proposal_store: PathBuf,
    pub checks: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfModifyApplyReport {
    pub at: String,
    pub operation_id: String,
    pub proposal_id: String,
    pub approval_id: String,
    pub target_path: PathBuf,
    pub dry_run_report: SelfModifyDryRunReport,
    pub check_profile: SelfModifyCheckProfile,
    pub pre_heartbeat: SelfModifyHeartbeatReport,
    pub pre_checks: Vec<SelfModifyCheckReport>,
    pub patch_report: PatchApplyReport,
    pub post_heartbeat: SelfModifyHeartbeatReport,
    pub post_checks: Vec<SelfModifyCheckReport>,
    pub post_checks_passed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_rollback: Option<SelfModifyRollbackReport>,
    pub rollback_snapshot: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfModifyCheckProfile {
    pub change_kind: SelfModifyChangeKind,
    pub source: String,
    pub commands: Vec<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfModifyCheckReport {
    pub command: String,
    pub status: i32,
    pub passed: bool,
    pub analysis: TestFailureAnalysis,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfModifyRollbackReport {
    pub at: String,
    pub operation_id: String,
    pub proposal_id: String,
    pub target_path: PathBuf,
    pub snapshot_path: PathBuf,
    pub restored_snapshot: bool,
    pub removed_created_target: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelfModifyOperationKind {
    Apply,
    Rollback,
    AutoRollback,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SelfModifyOperationRecord {
    pub id: String,
    pub at: String,
    pub kind: SelfModifyOperationKind,
    pub proposal_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_id: Option<String>,
    pub target_path: PathBuf,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check_profile: Option<SelfModifyCheckProfile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_checks_passed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_rollback_operation_id: Option<String>,
}
