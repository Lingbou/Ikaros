// SPDX-License-Identifier: GPL-3.0-only

use super::{audit::append_schedule_delivery_audit, types::ScheduleDeliveryReport};
use ikaros_automation::{
    LocalScheduleStore, ScheduleDeliveryTarget, ScheduleRunUpdate, ScheduledJob,
};
use ikaros_core::{IkarosError, IkarosPaths, Result, TaskState, redact_secrets};
use ikaros_gateway::LocalGatewayStore;
use ikaros_harness::TaskExecutionReport;
use serde_json::json;

pub(super) fn deliver_scheduled_job(
    job: &ScheduledJob,
    task_state: &TaskState,
    summary: &str,
    update: Option<&ScheduleRunUpdate>,
    task_report: Option<&TaskExecutionReport>,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
) -> Vec<ScheduleDeliveryReport> {
    let targets = if job.deliveries.is_empty() {
        ScheduleDeliveryTarget::default_targets()
    } else {
        job.deliveries.clone()
    };
    let run_id = task_report
        .map(|report| report.task_id.clone())
        .or_else(|| update.map(|update| update.ran_at.clone()))
        .unwrap_or_else(|| "run".into());
    let content = scheduled_job_delivery_content(job, task_state, summary, update, task_report);

    targets
        .into_iter()
        .map(|target| {
            deliver_scheduled_job_to_target(target, job, &run_id, &content, store, paths)
                .unwrap_or_else(|error| failed_delivery_report(target, job, paths, error))
        })
        .collect()
}

fn deliver_scheduled_job_to_target(
    target: ScheduleDeliveryTarget,
    job: &ScheduledJob,
    run_id: &str,
    content: &str,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
) -> Result<ScheduleDeliveryReport> {
    match target {
        ScheduleDeliveryTarget::LocalFile => {
            let path = store.write_local_delivery(&job.id, run_id, content)?;
            append_schedule_delivery_audit(
                paths,
                "schedule_delivery",
                "scheduled job delivered",
                json!({
                    "job_id": &job.id,
                    "target": target.as_str(),
                    "path": path.display().to_string(),
                }),
            )?;
            Ok(ScheduleDeliveryReport {
                target,
                status: "Delivered".into(),
                summary: format!("wrote local delivery {}", path.display()),
                path: Some(path),
                delivery_id: None,
            })
        }
        ScheduleDeliveryTarget::GatewayOutbox => {
            let gateway = LocalGatewayStore::new(&paths.gateway_dir);
            let delivery = gateway.deliver(&job.id, "schedule_report", content)?;
            append_schedule_delivery_audit(
                paths,
                "schedule_delivery",
                "scheduled job delivered",
                json!({
                    "job_id": &job.id,
                    "target": target.as_str(),
                    "delivery_id": &delivery.id,
                    "outbox": gateway.outbox_path().display().to_string(),
                }),
            )?;
            Ok(ScheduleDeliveryReport {
                target,
                status: "Delivered".into(),
                summary: format!("wrote gateway delivery {}", delivery.id),
                path: Some(gateway.outbox_path().to_path_buf()),
                delivery_id: Some(delivery.id),
            })
        }
    }
}

fn failed_delivery_report(
    target: ScheduleDeliveryTarget,
    job: &ScheduledJob,
    paths: &IkarosPaths,
    error: IkarosError,
) -> ScheduleDeliveryReport {
    let summary = redact_secrets(&error.to_string());
    let _ = append_schedule_delivery_audit(
        paths,
        "schedule_delivery_failed",
        "scheduled job delivery failed",
        json!({
            "job_id": &job.id,
            "target": target.as_str(),
            "summary": &summary,
        }),
    );
    ScheduleDeliveryReport {
        target,
        status: "Failed".into(),
        summary,
        path: None,
        delivery_id: None,
    }
}

fn scheduled_job_delivery_content(
    job: &ScheduledJob,
    task_state: &TaskState,
    summary: &str,
    update: Option<&ScheduleRunUpdate>,
    task_report: Option<&TaskExecutionReport>,
) -> String {
    let mut lines = vec![
        "# Ikaros Scheduled Job Result".to_string(),
        String::new(),
        format!("- job_id: {}", job.id),
        format!("- title: {}", job.title),
        format!("- state: {task_state:?}"),
        format!("- summary: {}", redact_secrets(summary)),
    ];
    if let Some(agent) = &job.agent {
        lines.push(format!("- agent: {agent}"));
    }
    if let Some(update) = update {
        lines.push(format!("- ran_at: {}", update.ran_at));
        lines.push(format!("- enabled: {}", update.enabled));
        if let Some(next_run_at) = &update.next_run_at {
            lines.push(format!("- next_run_at: {next_run_at}"));
        }
    }
    if let Some(report) = task_report {
        lines.push(format!("- task_id: {}", report.task_id));
        lines.push(format!("- steps: {}", report.steps.len()));
    }
    lines.push(String::new());
    lines.join("\n")
}
