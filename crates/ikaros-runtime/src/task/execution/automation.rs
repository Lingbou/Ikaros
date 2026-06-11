// SPDX-License-Identifier: GPL-3.0-only

use super::deterministic::execute_task_text;
use ikaros_core::{IkarosPaths, Result};
use ikaros_harness::TaskExecutionReport;
use std::path::Path;

pub async fn execute_task_for_automation(
    task_text: String,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<TaskExecutionReport> {
    Ok(
        execute_task_text(task_text, false, paths, workspace, agent_override)
            .await?
            .report,
    )
}
