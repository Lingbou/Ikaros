// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use clap::{Args, Subcommand};
use ikaros_automation::{LocalScheduleStore, ScheduleDeliveryTarget, ScheduledJob};
use ikaros_core::IkarosPaths;
use ikaros_runtime::{run_due_jobs, run_schedule_worker_tick};
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
            let job = store.add_with_deliveries(
                args.task,
                args.at,
                args.every_seconds,
                agent,
                deliveries,
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
            let reports = run_due_jobs(jobs, &store, paths, workspace, agent_override).await?;
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
        let report =
            run_schedule_worker_tick(store, args.limit, paths, workspace, agent_override).await?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        if args.once {
            break;
        }
        sleep(Duration::from_secs(args.interval_seconds)).await;
    }
    Ok(())
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
