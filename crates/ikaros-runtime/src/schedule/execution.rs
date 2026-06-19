// SPDX-License-Identifier: GPL-3.0-only

use super::{delivery::deliver_scheduled_job, types::ScheduledJobRunReport};
use crate::session::{
    RuntimeSessionEntryInput, active_leaf_entry_id, append_runtime_session_entry,
    append_runtime_session_event, delivery_payload, runtime_session_target, schedule_session_id,
    schedule_session_source, schedule_turn_id, upsert_runtime_session,
};
use crate::{TaskRunOptions, execute_task_text_with_options, task_report_summary};
use ikaros_automation::{LocalScheduleStore, ScheduledJob};
use ikaros_core::{IkarosPaths, Result, TaskState, redact_secrets};
use ikaros_harness::TaskExecutionReport;
use ikaros_session::{AgentEventKind, AgentEventSource, SessionEntryKind};
use serde_json::json;
use std::path::Path;

pub async fn run_scheduled_job(
    job: ScheduledJob,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ScheduledJobRunReport> {
    let agent = job.agent.as_deref().or(agent_override);
    let session_id = schedule_session_id(&job.id);
    let turn_id = schedule_turn_id(&job.id);
    let task_options = TaskRunOptions::agent_loop(false).with_session(
        session_id.to_string(),
        turn_id.to_string(),
        schedule_session_source(&job.id),
    );
    match execute_task_text_with_options(job.task.clone(), task_options, paths, workspace, agent)
        .await
    {
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
                paths,
                workspace,
                agent,
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
                paths,
                workspace,
                agent,
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
    paths: &'a IkarosPaths,
    workspace: &'a Path,
    agent: Option<&'a str>,
    job: &'a ScheduledJob,
    task_state: &'a TaskState,
    summary: &'a str,
    update: Option<&'a ikaros_automation::ScheduleRunUpdate>,
    deliveries: &'a [super::types::ScheduleDeliveryReport],
    task_report: Option<&'a TaskExecutionReport>,
}

fn record_scheduled_job_session(input: ScheduledJobSessionInput<'_>) -> Result<()> {
    let target = runtime_session_target(input.paths, input.workspace, input.agent)?;
    let session_id = schedule_session_id(&input.job.id);
    upsert_runtime_session(&target, &session_id, schedule_session_source(&input.job.id))?;
    let run_id = input
        .task_report
        .map(|report| report.task_id.as_str())
        .or_else(|| input.update.map(|update| update.ran_at.as_str()))
        .unwrap_or(input.job.id.as_str());
    let turn_id = schedule_turn_id(&input.job.id);
    append_runtime_session_event(
        &target,
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
        &target,
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
    let parent_entry_id = active_leaf_entry_id(&target, &session_id)?;
    let user_entry_id = append_runtime_session_entry(RuntimeSessionEntryInput {
        target: &target,
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
        &target,
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
        target: &target,
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
                "deliveries": deliveries,
            })),
        ),
    })?;
    if input.task_state == &TaskState::Failed {
        append_runtime_session_event(
            &target,
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
        &target,
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
