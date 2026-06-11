// SPDX-License-Identifier: GPL-3.0-only

use crate::{Result, now_rfc3339, redact_json, redact_secrets};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskState {
    Created,
    Planning,
    WaitingForApproval,
    Running,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RiskLevel {
    SafeRead,
    LocalWrite,
    ShellRead,
    ShellWrite,
    Network,
    DatabaseWrite,
    RemoteServer,
    Destructive,
    SecretAccess,
    SelfModify,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    AskUser,
    Deny,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub state: TaskState,
    pub created_at: String,
}

impl Task {
    pub fn new(title: impl Into<String>) -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            title: title.into(),
            state: TaskState::Created,
            created_at: now_rfc3339()?,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Plan {
    pub task_id: String,
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanStep {
    pub id: String,
    pub description: String,
    pub risk: RiskLevel,
    pub tool: Option<String>,
}

impl PlanStep {
    pub fn new(description: impl Into<String>, risk: RiskLevel, tool: Option<String>) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            description: description.into(),
            risk,
            tool,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub risk: RiskLevel,
    pub input: serde_json::Value,
}

impl ToolCall {
    pub fn new(name: impl Into<String>, risk: RiskLevel, input: serde_json::Value) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            risk,
            input,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResult {
    pub call_id: String,
    pub ok: bool,
    pub output: serde_json::Value,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeEvent {
    pub id: String,
    pub at: String,
    pub task_id: Option<String>,
    pub kind: String,
    pub message: String,
    pub data: serde_json::Value,
}

impl RuntimeEvent {
    pub fn new(
        kind: impl Into<String>,
        message: impl Into<String>,
        data: serde_json::Value,
    ) -> Result<Self> {
        Ok(Self {
            id: Uuid::new_v4().to_string(),
            at: now_rfc3339()?,
            task_id: None,
            kind: kind.into(),
            message: redact_secrets(&message.into()),
            data: redact_json(data),
        })
    }

    pub fn for_task(mut self, task_id: impl Into<String>) -> Self {
        self.task_id = Some(task_id.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskRunnerReport {
    pub task_id: String,
    pub state: TaskState,
    pub plan_summary: Vec<String>,
    pub audit_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeCoordinator;

impl RuntimeCoordinator {
    pub fn create_plan(task: &Task) -> Plan {
        Plan {
            task_id: task.id.clone(),
            steps: vec![
                PlanStep::new(
                    "Load persona/config/memory and build redacted context.",
                    RiskLevel::SafeRead,
                    Some("persona_load".into()),
                ),
                PlanStep::new(
                    "Search local memory and local RAG for relevant context.",
                    RiskLevel::SafeRead,
                    Some("memory_search".into()),
                ),
                PlanStep::new(
                    "Execute safe harness skills and ask for approval on risky writes.",
                    RiskLevel::LocalWrite,
                    None,
                ),
                PlanStep::new(
                    "Write audit event and summarize result without committing.",
                    RiskLevel::SafeRead,
                    Some("task_summarize".into()),
                ),
            ],
        }
    }
}
