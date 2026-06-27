// SPDX-License-Identifier: GPL-3.0-only

use crate::agent_loop::AgentLoopReport;
use ikaros_core::{AgentMode, PolicyDecision, TaskState};
use ikaros_harness::TaskExecutionReport;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentHandoffReport {
    pub agent: String,
    pub mode: AgentMode,
    pub task_id: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub dry_run: bool,
    pub agent_loop: bool,
    pub policy_decisions: Vec<PolicyDecision>,
    pub audit_path: PathBuf,
    pub report: TaskExecutionReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_report: Option<AgentLoopReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentPoolTask {
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

impl AgentPoolTask {
    pub fn new(task: impl Into<String>, profile: Option<String>) -> Self {
        Self {
            task: task.into(),
            profile,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentPoolItemReport {
    pub index: usize,
    pub task: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<TaskState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<AgentHandoffReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentPoolReport {
    pub dry_run: bool,
    pub agent_loop: bool,
    pub concurrency: usize,
    pub total: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub reports: Vec<AgentPoolItemReport>,
}
