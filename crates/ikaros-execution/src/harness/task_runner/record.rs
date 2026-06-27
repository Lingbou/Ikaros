// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{Result, RiskLevel, TaskState, now_rfc3339, redact_secrets};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PlanStepStatus {
    Pending,
    Running,
    WaitingForApproval,
    Succeeded,
    Failed,
    Skipped,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExecutablePlanStep {
    pub id: String,
    pub description: String,
    pub skill: String,
    pub input: serde_json::Value,
    pub risk: RiskLevel,
}

impl ExecutablePlanStep {
    pub fn new(
        description: impl Into<String>,
        skill: impl Into<String>,
        input: serde_json::Value,
        risk: RiskLevel,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            description: description.into(),
            skill: skill.into(),
            input,
            risk,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StepExecutionRecord {
    pub step_id: String,
    pub description: String,
    pub skill: String,
    pub risk: RiskLevel,
    pub status: PlanStepStatus,
    pub attempts: u32,
    pub summary: String,
    pub approval_id: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

impl StepExecutionRecord {
    pub(super) fn pending(step: &ExecutablePlanStep) -> Self {
        Self {
            step_id: step.id.clone(),
            description: step.description.clone(),
            skill: step.skill.clone(),
            risk: step.risk.clone(),
            status: PlanStepStatus::Pending,
            attempts: 0,
            summary: String::new(),
            approval_id: None,
            started_at: None,
            completed_at: None,
        }
    }

    pub(super) fn start(&mut self) -> Result<()> {
        self.status = PlanStepStatus::Running;
        self.summary = "running".into();
        self.started_at = Some(now_rfc3339()?);
        Ok(())
    }

    pub(super) fn complete(
        &mut self,
        status: PlanStepStatus,
        summary: impl Into<String>,
        approval_id: Option<String>,
    ) -> Result<()> {
        self.status = status;
        self.summary = redact_secrets(&summary.into());
        self.approval_id = approval_id;
        self.completed_at = Some(now_rfc3339()?);
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskExecutionReport {
    pub task_id: String,
    pub state: TaskState,
    pub steps: Vec<StepExecutionRecord>,
    pub audit_path: Option<PathBuf>,
}
