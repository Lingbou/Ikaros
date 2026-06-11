// SPDX-License-Identifier: GPL-3.0-only

use ikaros_automation::{ScheduleDeliveryTarget, ScheduleRunUpdate};
use ikaros_core::TaskState;
use ikaros_harness::TaskExecutionReport;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ScheduleDeliveryReport {
    pub target: ScheduleDeliveryTarget,
    pub status: String,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduledJobRunReport {
    pub job_id: String,
    pub title: String,
    pub task_state: TaskState,
    pub summary: String,
    pub update: Option<ScheduleRunUpdate>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deliveries: Vec<ScheduleDeliveryReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_report: Option<TaskExecutionReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduleWorkerTickReport {
    pub kind: String,
    pub due: usize,
    pub ran: usize,
    pub reports: Vec<ScheduledJobRunReport>,
}
