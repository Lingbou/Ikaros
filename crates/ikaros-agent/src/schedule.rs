// SPDX-License-Identifier: GPL-3.0-only

use crate::task_loop::{
    TaskExecutionContext, TaskRunOptions, execute_task_text_with_context, task_report_summary,
};
use ikaros_core::{IkarosError, IkarosPaths, Result, TaskState, redact_secrets};
use ikaros_execution::harness::{AuditEvent, AuditLog, TaskExecutionReport};
use ikaros_state::automation::{
    LocalScheduleStore, ScheduleDeliveryTarget, ScheduleRunUpdate, ScheduledJob,
};
use ikaros_state::gateway::LocalGatewayStore;
use ikaros_state::session::{
    AgentEventKind, AgentEventSource, RuntimeSessionEntryInput, RuntimeSessionTarget,
    SessionEntryKind, active_leaf_entry_id, append_runtime_session_entry,
    append_runtime_session_event, delivery_payload, schedule_session_id, schedule_session_source,
    schedule_turn_id, upsert_runtime_session,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

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

pub struct ScheduledJobExecutionContext<'a> {
    pub paths: &'a IkarosPaths,
    pub workspace: &'a Path,
    pub agent_label: Option<&'a str>,
    pub session_target: &'a RuntimeSessionTarget,
    pub task: TaskExecutionContext<'a>,
}

pub async fn run_scheduled_job_with_context(
    job: ScheduledJob,
    store: &LocalScheduleStore,
    context: ScheduledJobExecutionContext<'_>,
) -> Result<ScheduledJobRunReport> {
    let task_options = TaskRunOptions::deterministic(false).with_session(
        schedule_session_id(&job.id).to_string(),
        schedule_turn_id(&job.id).to_string(),
        schedule_session_source(&job.id),
    );
    let ScheduledJobExecutionContext {
        paths,
        workspace,
        agent_label,
        session_target,
        task,
    } = context;
    let execution = execute_task_text_with_context(job.task.clone(), task_options, task).await;
    complete_scheduled_job_run(
        job,
        store,
        paths,
        workspace,
        agent_label,
        session_target,
        execution,
    )
}

pub fn record_scheduled_job_preflight_failure(
    job: ScheduledJob,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_label: Option<&str>,
    session_target: &RuntimeSessionTarget,
    error: IkarosError,
) -> Result<ScheduledJobRunReport> {
    complete_scheduled_job_run(
        job,
        store,
        paths,
        workspace,
        agent_label,
        session_target,
        Err(error),
    )
}

fn complete_scheduled_job_run(
    job: ScheduledJob,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_label: Option<&str>,
    session_target: &RuntimeSessionTarget,
    execution: Result<crate::task_loop::RuntimeTaskExecution>,
) -> Result<ScheduledJobRunReport> {
    match execution {
        Ok(execution) => {
            let report = execution.report;
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
            record_scheduled_job_session(ScheduledJobSessionInput {
                target: session_target,
                workspace,
                agent: agent_label,
                job: &job,
                task_state: &report.state,
                summary: &summary,
                update: update.as_ref(),
                deliveries: &deliveries,
                task_report: Some(&report),
            })?;
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
            record_scheduled_job_session(ScheduledJobSessionInput {
                target: session_target,
                workspace,
                agent: agent_label,
                job: &job,
                task_state: &TaskState::Failed,
                summary: &summary,
                update: update.as_ref(),
                deliveries: &deliveries,
                task_report: None,
            })?;
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

struct ScheduledJobSessionInput<'a> {
    target: &'a RuntimeSessionTarget,
    workspace: &'a Path,
    agent: Option<&'a str>,
    job: &'a ScheduledJob,
    task_state: &'a TaskState,
    summary: &'a str,
    update: Option<&'a ScheduleRunUpdate>,
    deliveries: &'a [ScheduleDeliveryReport],
    task_report: Option<&'a TaskExecutionReport>,
}

fn record_scheduled_job_session(input: ScheduledJobSessionInput<'_>) -> Result<()> {
    let session_id = schedule_session_id(&input.job.id);
    upsert_runtime_session(
        input.target,
        &session_id,
        schedule_session_source(&input.job.id),
    )?;
    let run_id = input
        .task_report
        .map(|report| report.task_id.as_str())
        .or_else(|| input.update.map(|update| update.ran_at.as_str()))
        .unwrap_or(input.job.id.as_str());
    let turn_id = schedule_turn_id(&input.job.id);
    append_runtime_session_event(
        input.target,
        &session_id,
        &turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::SessionStart,
        json!({
            "source": "schedule",
            "job_id": &input.job.id,
            "title": &input.job.title,
        }),
    )?;
    append_runtime_session_event(
        input.target,
        &session_id,
        &turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnStart,
        json!({
            "source": "schedule",
            "job_id": &input.job.id,
            "title": &input.job.title,
            "run_id": run_id,
        }),
    )?;
    let parent_entry_id = active_leaf_entry_id(input.target, &session_id)?;
    let user_entry_id = append_runtime_session_entry(RuntimeSessionEntryInput {
        target: input.target,
        session_id: &session_id,
        parent_entry_id,
        turn_id: &turn_id,
        kind: SessionEntryKind::UserMessage,
        visible_text: Some(redact_secrets(&input.job.task)),
        payload: json!({
            "role": "user",
            "source": "schedule",
            "job_id": &input.job.id,
            "title": &input.job.title,
            "content": redact_secrets(&input.job.task),
        }),
    })?;
    append_runtime_session_event(
        input.target,
        &session_id,
        &turn_id,
        AgentEventSource::User,
        AgentEventKind::UserMessage,
        json!({
            "content": redact_secrets(&input.job.task),
        }),
    )?;
    let deliveries = input
        .deliveries
        .iter()
        .map(|delivery| {
            json!({
                "target": delivery.target.as_str(),
                "status": &delivery.status,
                "summary": &delivery.summary,
                "path": delivery.path.as_ref().map(|path| path.display().to_string()),
                "delivery_id": &delivery.delivery_id,
            })
        })
        .collect::<Vec<_>>();
    append_runtime_session_entry(RuntimeSessionEntryInput {
        target: input.target,
        session_id: &session_id,
        parent_entry_id: Some(user_entry_id),
        turn_id: &turn_id,
        kind: SessionEntryKind::Custom,
        visible_text: Some(redact_secrets(input.summary)),
        payload: delivery_payload(
            "schedule_run",
            schedule_status_str(input.task_state),
            input.summary,
            Some(json!({
                "job_id": &input.job.id,
                "title": &input.job.title,
                "run_id": run_id,
                "task_id": input.task_report.map(|report| report.task_id.as_str()),
                "task_state": format!("{:?}", input.task_state),
                "step_count": input.task_report.map(|report| report.steps.len()),
                "enabled": input.update.map(|update| update.enabled),
                "next_run_at": input.update.and_then(|update| update.next_run_at.as_deref()),
                "workspace": input.workspace.display().to_string(),
                "agent": input.agent,
                "deliveries": deliveries,
            })),
        ),
    })?;
    if input.task_state == &TaskState::Failed {
        append_runtime_session_event(
            input.target,
            &session_id,
            &turn_id,
            AgentEventSource::Runtime,
            AgentEventKind::Error,
            json!({
                "source": "schedule",
                "job_id": &input.job.id,
                "summary": redact_secrets(input.summary),
            }),
        )?;
    }
    append_runtime_session_event(
        input.target,
        &session_id,
        &turn_id,
        AgentEventSource::Runtime,
        AgentEventKind::TurnEnd,
        json!({
            "source": "schedule",
            "job_id": &input.job.id,
            "status": schedule_status_str(input.task_state),
        }),
    )
}

fn deliver_scheduled_job(
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

fn append_schedule_delivery_audit(
    paths: &IkarosPaths,
    kind: &str,
    message: &str,
    data: serde_json::Value,
) -> Result<()> {
    AuditLog::new(&paths.audit_dir).append(AuditEvent::new(kind, None, message, data)?)
}

fn schedule_status_str(state: &TaskState) -> &'static str {
    match state {
        TaskState::Completed => "completed",
        TaskState::Failed => "failed",
        TaskState::WaitingForApproval => "waiting_for_approval",
        TaskState::Blocked => "blocked",
        TaskState::Cancelled => "cancelled",
        TaskState::Created => "created",
        TaskState::Planning => "planning",
        TaskState::Running => "running",
    }
}
