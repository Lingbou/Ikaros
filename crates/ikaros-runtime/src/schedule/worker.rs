// SPDX-License-Identifier: GPL-3.0-only

use super::{
    execution::run_scheduled_job,
    types::{ScheduleWorkerTickReport, ScheduledJobRunReport},
};
use ikaros_automation::{LocalScheduleStore, ScheduledJob};
use ikaros_core::{IkarosError, IkarosPaths, Result, TaskState, redact_secrets};
use std::path::Path;

pub async fn run_schedule_worker_tick(
    store: &LocalScheduleStore,
    limit: usize,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ScheduleWorkerTickReport> {
    if limit == 0 {
        return Err(IkarosError::Message(
            "schedule worker limit must be greater than zero".into(),
        ));
    }
    let mut jobs = store.due_now()?;
    jobs.truncate(limit);
    let due = jobs.len();
    let reports = run_due_jobs(jobs, store, paths, workspace, agent_override).await?;
    Ok(ScheduleWorkerTickReport {
        kind: "schedule_worker_tick".into(),
        due,
        ran: reports.len(),
        reports,
    })
}

pub async fn run_due_jobs(
    jobs: Vec<ScheduledJob>,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Vec<super::types::ScheduledJobRunReport>> {
    let mut reports = Vec::new();
    for job in jobs {
        match run_scheduled_job(job.clone(), store, paths, workspace, agent_override).await {
            Ok(report) => reports.push(report),
            Err(error) => reports.push(failed_scheduled_job_report(job, error)),
        }
    }
    Ok(reports)
}

fn failed_scheduled_job_report(job: ScheduledJob, error: IkarosError) -> ScheduledJobRunReport {
    ScheduledJobRunReport {
        job_id: job.id,
        title: job.title,
        task_state: TaskState::Failed,
        summary: redact_secrets(&error.to_string()),
        update: None,
        deliveries: Vec::new(),
        task_report: None,
    }
}
