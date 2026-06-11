// SPDX-License-Identifier: GPL-3.0-only

use crate::AgentLoopReport;
use ikaros_core::{Plan, PolicyDecision, Task};
use ikaros_harness::{ExecutablePlanStep, TaskExecutionReport};
use ikaros_soul::EmotionState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTaskPlan {
    pub plan: Plan,
    pub executable_steps: Vec<ExecutablePlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuntimeTaskExecution {
    pub task: Task,
    pub plan: Plan,
    pub report: TaskExecutionReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_loop: Option<AgentLoopReport>,
    pub dry_run: bool,
    pub agent: Option<String>,
    pub final_emotion: EmotionState,
    pub policy_decisions: Vec<PolicyDecision>,
    pub audit_path: PathBuf,
    pub approvals_path: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct TaskRunOptions {
    pub dry_run: bool,
    pub agent_loop: bool,
    pub loop_max_iterations: u32,
}

impl TaskRunOptions {
    pub fn deterministic(dry_run: bool) -> Self {
        Self {
            dry_run,
            agent_loop: false,
            ..Self::default()
        }
    }

    pub fn agent_loop(dry_run: bool) -> Self {
        Self {
            dry_run,
            agent_loop: true,
            ..Self::default()
        }
    }
}

impl Default for TaskRunOptions {
    fn default() -> Self {
        Self {
            dry_run: false,
            agent_loop: false,
            loop_max_iterations: 6,
        }
    }
}
