// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_agent::schedule::{
    ScheduleWorkerTickReport, ScheduledJobExecutionContext, ScheduledJobRunReport,
    record_scheduled_job_preflight_failure, run_scheduled_job_with_context,
};
use ikaros_agent::soul::load_or_default;
use ikaros_agent::task_loop::{TaskAgentLoopContext, TaskExecutionContext};
use ikaros_core::{IkarosError, IkarosPaths, TaskState, redact_secrets};
use ikaros_host::{RuntimeHarness, runtime_harness, runtime_harness_model_provider};
use ikaros_state::automation::{
    LocalScheduleStore, ScheduleDeliveryTarget, ScheduleJobOptions, ScheduleRetryPolicy,
    ScheduledJob,
};
use ikaros_state::session::{RuntimeSessionTarget, SqliteSessionStore};
use std::{path::Path, time::Duration};
use tokio::time::sleep;

#[derive(Debug, Subcommand)]
pub(crate) enum ScheduleCommand {
    Add(ScheduleAdd),
    List {
        #[arg(long)]
        all: bool,
    },
    RunDue {
        #[arg(long, default_value_t = 10)]
        limit: usize,
        #[arg(long)]
        dry_run: bool,
    },
    Worker(ScheduleWorker),
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    Delete {
        id: String,
    },
}

#[derive(Debug, Args)]
pub(crate) struct ScheduleAdd {
    task: String,
    #[arg(long, default_value = "now", help = "RFC3339 timestamp or 'now'")]
    at: String,
    #[arg(long)]
    every_seconds: Option<u64>,
    #[arg(long, value_name = "PROFILE")]
    profile: Option<String>,
    #[arg(
        long = "delivery",
        value_name = "TARGET",
        help = "Delivery target: local-file or gateway-outbox; repeat to use multiple targets"
    )]
    delivery: Vec<String>,
    #[arg(long = "retry-max-attempts", default_value_t = 1)]
    retry_max_attempts: u32,
    #[arg(long = "retry-backoff-seconds", default_value_t = 60)]
    retry_backoff_seconds: u64,
    #[arg(long = "grace-period-seconds")]
    grace_period_seconds: Option<u64>,
    #[arg(long)]
    timezone: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct ScheduleWorker {
    #[arg(long = "interval-seconds", default_value_t = 60)]
    interval_seconds: u64,
    #[arg(long, default_value_t = 10)]
    limit: usize,
    #[arg(long)]
    once: bool,
}

pub(crate) async fn schedule_command(
    command: ScheduleCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let store = LocalScheduleStore::new(&paths.automation_dir);
    match command {
        ScheduleCommand::Add(args) => {
            let agent = args
                .profile
                .or_else(|| agent_override.map(ToOwned::to_owned));
            let deliveries = parse_delivery_targets(args.delivery)?;
            let job = store.add_with_options(
                args.task,
                args.at,
                ScheduleJobOptions {
                    interval_seconds: args.every_seconds,
                    agent,
                    deliveries,
                    retry: ScheduleRetryPolicy {
                        max_attempts: args.retry_max_attempts,
                        backoff_seconds: args.retry_backoff_seconds,
                    },
                    grace_period_seconds: args.grace_period_seconds,
                    timezone: args.timezone,
                },
            )?;
            print_job("scheduled", &job)?;
        }
        ScheduleCommand::List { all } => {
            let jobs = store
                .list()?
                .into_iter()
                .filter(|job| all || job.enabled)
                .collect::<Vec<_>>();
            println!("{}", serde_json::to_string_pretty(&jobs)?);
            println!("schedule_store: {}", store.path().display());
        }
        ScheduleCommand::RunDue { limit, dry_run } => {
            let mut jobs = store.due_now()?;
            jobs.truncate(limit);
            if dry_run {
                println!("{}", serde_json::to_string_pretty(&jobs)?);
                println!("schedule_store: {}", store.path().display());
                return Ok(());
            }
            let reports =
                run_due_jobs_with_host_context(jobs, &store, paths, workspace, agent_override)
                    .await?;
            println!("{}", serde_json::to_string_pretty(&reports)?);
            println!("schedule_store: {}", store.path().display());
        }
        ScheduleCommand::Worker(args) => {
            run_schedule_worker(args, &store, paths, workspace, agent_override).await?;
        }
        ScheduleCommand::Enable { id } => match store.set_enabled(&id, true)? {
            Some(job) => print_job("enabled", &job)?,
            None => anyhow::bail!("scheduled job not found: {id}"),
        },
        ScheduleCommand::Disable { id } => match store.set_enabled(&id, false)? {
            Some(job) => print_job("disabled", &job)?,
            None => anyhow::bail!("scheduled job not found: {id}"),
        },
        ScheduleCommand::Delete { id } => {
            let deleted = store.delete(&id)?;
            println!("deleted: {deleted}");
            println!("schedule_store: {}", store.path().display());
        }
    }
    Ok(())
}

async fn run_schedule_worker(
    args: ScheduleWorker,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    if args.interval_seconds == 0 {
        anyhow::bail!("schedule worker interval must be greater than zero");
    }
    if args.limit == 0 {
        anyhow::bail!("schedule worker limit must be greater than zero");
    }
    println!("schedule_worker: started");
    println!("interval_seconds: {}", args.interval_seconds);
    println!("limit: {}", args.limit);
    println!("schedule_store: {}", store.path().display());
    loop {
        let report = run_schedule_worker_tick_with_host_context(
            store,
            args.limit,
            paths,
            workspace,
            agent_override,
        )
        .await?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if args.once {
            break;
        }
        sleep(Duration::from_secs(args.interval_seconds)).await;
    }
    Ok(())
}

async fn run_schedule_worker_tick_with_host_context(
    store: &LocalScheduleStore,
    limit: usize,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ScheduleWorkerTickReport> {
    if limit == 0 {
        anyhow::bail!("schedule worker limit must be greater than zero");
    }
    let mut jobs = store.due_now()?;
    jobs.truncate(limit);
    let due = jobs.len();
    let reports =
        run_due_jobs_with_host_context(jobs, store, paths, workspace, agent_override).await?;
    Ok(ScheduleWorkerTickReport {
        kind: "schedule_worker_tick".into(),
        due,
        ran: reports.len(),
        reports,
    })
}

async fn run_due_jobs_with_host_context(
    jobs: Vec<ScheduledJob>,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<Vec<ScheduledJobRunReport>> {
    let mut reports = Vec::new();
    for job in jobs {
        match run_scheduled_job_with_host_context(
            job.clone(),
            store,
            paths,
            workspace,
            agent_override,
        )
        .await
        {
            Ok(report) => reports.push(report),
            Err(error) => reports.push(failed_scheduled_job_report(job, error)),
        }
    }
    Ok(reports)
}

async fn run_scheduled_job_with_host_context(
    job: ScheduledJob,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<ScheduledJobRunReport> {
    let effective_agent = job
        .agent
        .clone()
        .or_else(|| agent_override.map(ToOwned::to_owned));
    let agent_label = effective_agent.as_deref();
    let harness = match runtime_harness(paths, workspace, agent_label) {
        Ok(harness) => harness,
        Err(error) => {
            let session_target = fallback_schedule_session_target(paths, workspace, agent_label);
            return record_scheduled_job_setup_failure(
                job,
                store,
                paths,
                workspace,
                agent_label,
                &session_target,
                error,
            );
        }
    };
    let session_target = RuntimeSessionTarget {
        store: SqliteSessionStore::new(harness.agent_instance.state_dir.clone()),
        agent_id: harness.agent_instance.agent_id.clone(),
        workspace: harness.agent_instance.workspace.clone(),
    };
    let persona = match load_or_default(&paths.persona_dir) {
        Ok(persona) => persona,
        Err(error) => {
            return record_scheduled_job_setup_failure(
                job,
                store,
                paths,
                &harness.agent_instance.workspace,
                agent_label,
                &session_target,
                error,
            );
        }
    };
    let provider = match runtime_harness_model_provider(paths, &harness) {
        Ok(provider) => provider,
        Err(error) => {
            return record_scheduled_job_setup_failure(
                job,
                store,
                paths,
                &harness.agent_instance.workspace,
                agent_label,
                &session_target,
                error,
            );
        }
    };
    let RuntimeHarness {
        agent,
        agent_instance,
        session,
        registry,
        ..
    } = harness;
    let task_context = TaskExecutionContext {
        agent: &agent,
        session,
        registry,
        agent_loop: Some(TaskAgentLoopContext {
            persona: &persona,
            provider: provider.as_ref(),
            session_target: &session_target,
        }),
    };
    Ok(run_scheduled_job_with_context(
        job,
        store,
        ScheduledJobExecutionContext {
            paths,
            workspace: &agent_instance.workspace,
            agent_label,
            session_target: &session_target,
            task: task_context,
        },
    )
    .await?)
}

fn record_scheduled_job_setup_failure(
    job: ScheduledJob,
    store: &LocalScheduleStore,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_label: Option<&str>,
    session_target: &RuntimeSessionTarget,
    error: IkarosError,
) -> Result<ScheduledJobRunReport> {
    Ok(record_scheduled_job_preflight_failure(
        job,
        store,
        paths,
        workspace,
        agent_label,
        session_target,
        error,
    )?)
}

fn failed_scheduled_job_report(job: ScheduledJob, error: anyhow::Error) -> ScheduledJobRunReport {
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

fn fallback_schedule_session_target(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_label: Option<&str>,
) -> RuntimeSessionTarget {
    let agent_id = sanitize_runtime_path_segment(agent_label.unwrap_or("build"));
    RuntimeSessionTarget {
        store: SqliteSessionStore::new(paths.home.join("agents").join(&agent_id)),
        agent_id,
        workspace: workspace.to_path_buf(),
    }
}

fn sanitize_runtime_path_segment(value: &str) -> String {
    let sanitized = redact_secrets(value).replace(['/', '\\', ':', '\n', '\r', '\t'], "_");
    if sanitized.trim().is_empty() {
        "build".into()
    } else {
        sanitized
    }
}

fn print_job(prefix: &str, job: &ScheduledJob) -> Result<()> {
    println!("{prefix}: {}", job.id);
    println!("{}", serde_json::to_string_pretty(job)?);
    Ok(())
}

fn parse_delivery_targets(values: Vec<String>) -> Result<Vec<ScheduleDeliveryTarget>> {
    if values.is_empty() {
        return Ok(ScheduleDeliveryTarget::default_targets());
    }
    values
        .into_iter()
        .map(|value| value.parse().map_err(Into::into))
        .collect()
}
