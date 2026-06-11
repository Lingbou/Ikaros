// SPDX-License-Identifier: GPL-3.0-only

use super::{delivery::deliver_scheduled_job, types::ScheduledJobRunReport};
use crate::{execute_task_for_automation, task_report_summary};
use ikaros_automation::{LocalScheduleStore, ScheduledJob};
use ikaros_core::{IkarosPaths, Result, TaskState, redact_secrets};
use std::path::Path;

pub async fn run_scheduled_job(
    job: ScheduledJob,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ScheduledJobRunReport> {
    let agent = job.agent.as_deref().or(agent_override);
    match execute_task_for_automation(job.task.clone(), paths, workspace, agent).await {
        Ok(report) => {
            let summary = task_report_summary(
                &report,
                format!("completed {} scheduled step(s)", report.steps.len()),
            );
            let update = store.record_run(&job.id, format!("{:?}", report.state), &summary)?;
            let deliveries = deliver_scheduled_job(
                &job,
                &report.state,
                &summary,
                update.as_ref(),
                Some(&report),
                store,
                paths,
            );
            Ok(ScheduledJobRunReport {
                job_id: job.id,
                title: job.title,
                task_state: report.state.clone(),
                summary,
                update,
                deliveries,
                task_report: Some(report),
            })
        }
        Err(error) => {
            let summary = redact_secrets(&error.to_string());
            let update = store.record_run(&job.id, "Failed", &summary)?;
            let deliveries = deliver_scheduled_job(
                &job,
                &TaskState::Failed,
                &summary,
                update.as_ref(),
                None,
                store,
                paths,
            );
            Ok(ScheduledJobRunReport {
                job_id: job.id,
                title: job.title,
                task_state: TaskState::Failed,
                summary,
                update,
                deliveries,
                task_report: None,
            })
        }
    }
}
