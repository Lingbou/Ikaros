// SPDX-License-Identifier: GPL-3.0-only

use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use ikaros_agent::agent_pool::{
    AgentHandoffContext, AgentHandoffReport, AgentPoolItemReport, AgentPoolReport, AgentPoolTask,
    agent_pool_report_from_items, pool_item_from_result, run_agent_handoff_with_options,
};
use ikaros_agent::soul::load_or_default;
use ikaros_agent::task_loop::{TaskAgentLoopContext, TaskRunOptions};
use ikaros_core::IkarosPaths;
use ikaros_host::{
    RuntimeHarness, agent_profile_report, agent_profiles_report, runtime_harness,
    runtime_harness_model_provider,
};
use ikaros_state::session::{RuntimeSessionTarget, SqliteSessionStore};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug, Subcommand)]
pub(crate) enum AgentCommand {
    List,
    Show { profile: Option<String> },
    Run(AgentRun),
    Batch(AgentBatch),
}

#[derive(Debug, Args)]
pub(crate) struct AgentRun {
    task: String,
    #[arg(long, value_name = "PROFILE")]
    profile: Option<String>,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    agent_loop: bool,
    #[arg(long, default_value_t = 6)]
    loop_max_iterations: u32,
    #[arg(long, value_name = "SESSION_ID")]
    parent_session: Option<String>,
}

#[derive(Debug, Args)]
pub(crate) struct AgentBatch {
    #[arg(long = "task", value_name = "TEXT")]
    tasks: Vec<String>,
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,
    #[arg(long, value_name = "PROFILE")]
    profile: Option<String>,
    #[arg(long, default_value_t = 2)]
    concurrency: usize,
    #[arg(long)]
    dry_run: bool,
    #[arg(long)]
    agent_loop: bool,
    #[arg(long, default_value_t = 6)]
    loop_max_iterations: u32,
    #[arg(long, value_name = "SESSION_ID")]
    parent_session: Option<String>,
}

pub(crate) async fn agent_command(
    command: AgentCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    match command {
        AgentCommand::List => {
            println!(
                "{}",
                serde_json::to_string_pretty(&agent_profiles_report(paths)?)?
            );
        }
        AgentCommand::Show { profile } => {
            let requested = profile.as_deref().or(agent_override);
            println!(
                "{}",
                serde_json::to_string_pretty(&agent_profile_report(paths, requested)?)?
            );
        }
        AgentCommand::Run(args) => {
            let requested = args.profile.as_deref().or(agent_override);
            let report = run_agent_handoff_with_host_context(
                paths,
                workspace,
                requested,
                args.task,
                TaskRunOptions {
                    dry_run: args.dry_run,
                    agent_loop: args.agent_loop,
                    loop_max_iterations: args.loop_max_iterations,
                    parent_session_id: args.parent_session,
                    ..TaskRunOptions::default()
                },
            )
            .await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "agent": report.agent,
                    "mode": report.mode,
                    "task_id": report.task_id,
                    "session_id": report.session_id,
                    "parent_session_id": report.parent_session_id,
                    "dry_run": report.dry_run,
                    "agent_loop": report.agent_loop,
                    "state": report.report.state,
                    "audit": report.audit_path,
                    "report": report.report,
                    "loop_report": report.loop_report,
                }))?
            );
        }
        AgentCommand::Batch(args) => {
            let default_profile = args.profile.as_deref().or(agent_override);
            let tasks = load_agent_batch_tasks(&args)?;
            let report = run_agent_pool_with_host_context(
                paths,
                workspace,
                tasks,
                default_profile,
                TaskRunOptions {
                    dry_run: args.dry_run,
                    agent_loop: args.agent_loop,
                    loop_max_iterations: args.loop_max_iterations,
                    parent_session_id: args.parent_session,
                    ..TaskRunOptions::default()
                },
                args.concurrency,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
    }
    Ok(())
}

async fn run_agent_handoff_with_host_context(
    paths: &IkarosPaths,
    workspace: &Path,
    profile: Option<&str>,
    task_text: impl Into<String>,
    options: TaskRunOptions,
) -> Result<AgentHandoffReport> {
    let harness = runtime_harness(paths, workspace, profile)?;
    let provider = if options.agent_loop {
        Some(runtime_harness_model_provider(paths, &harness)?)
    } else {
        None
    };
    let session_target = RuntimeSessionTarget {
        store: SqliteSessionStore::new(harness.agent_instance.state_dir.clone()),
        agent_id: harness.agent_instance.agent_id.clone(),
        workspace: harness.agent_instance.workspace.clone(),
    };
    let max_delegation_depth = harness.agent_instance.session_policy.max_delegation_depth;
    let RuntimeHarness {
        agent,
        session,
        registry,
        ..
    } = harness;
    if options.agent_loop {
        let persona = load_or_default(&paths.persona_dir)?;
        let provider = provider.expect("agent-loop provider");
        Ok(run_agent_handoff_with_options(
            task_text,
            options,
            AgentHandoffContext {
                agent: &agent,
                session,
                registry,
                agent_loop: Some(TaskAgentLoopContext {
                    persona: &persona,
                    provider: provider.as_ref(),
                    session_target: &session_target,
                }),
                parent_evidence_target: Some(&session_target),
                max_delegation_depth,
            },
        )
        .await?)
    } else {
        Ok(run_agent_handoff_with_options(
            task_text,
            options,
            AgentHandoffContext {
                agent: &agent,
                session,
                registry,
                agent_loop: None,
                parent_evidence_target: Some(&session_target),
                max_delegation_depth,
            },
        )
        .await?)
    }
}

async fn run_agent_pool_with_host_context(
    paths: &IkarosPaths,
    workspace: &Path,
    tasks: Vec<AgentPoolTask>,
    default_profile: Option<&str>,
    options: TaskRunOptions,
    concurrency: usize,
) -> Result<AgentPoolReport> {
    if tasks.is_empty() {
        anyhow::bail!("agent pool requires at least one task");
    }
    if concurrency == 0 {
        anyhow::bail!("agent pool concurrency must be greater than zero");
    }
    let concurrency = concurrency.min(tasks.len());
    let indexed_tasks = tasks.into_iter().enumerate().collect::<Vec<_>>();
    let mut reports = Vec::new();
    for chunk in indexed_tasks.chunks(concurrency) {
        let mut handles = Vec::new();
        for (index, task) in chunk.iter().cloned() {
            let paths = paths.clone();
            let workspace = workspace.to_path_buf();
            let default_profile = default_profile.map(ToOwned::to_owned);
            let task_text = task.task.clone();
            let requested_profile = task.profile.clone();
            let profile = requested_profile.clone().or(default_profile);
            let options = options.clone();
            let handle = tokio::spawn(async move {
                let result = run_agent_handoff_with_host_context(
                    &paths,
                    &workspace,
                    profile.as_deref(),
                    task_text.clone(),
                    options,
                )
                .await
                .map_err(|error| ikaros_core::IkarosError::Message(error.to_string()));
                pool_item_from_result(index, task_text, profile, result)
            });
            handles.push((index, task.task, requested_profile, handle));
        }
        for (index, task_text, profile, handle) in handles {
            match handle.await {
                Ok(report) => reports.push(report),
                Err(error) => reports.push(AgentPoolItemReport {
                    index,
                    task: ikaros_core::redact_secrets(&task_text),
                    profile,
                    ok: false,
                    state: None,
                    report: None,
                    error: Some(ikaros_core::redact_secrets(&format!(
                        "agent worker task join failed: {error}"
                    ))),
                }),
            }
        }
    }
    reports.sort_by_key(|report| report.index);
    Ok(agent_pool_report_from_items(&options, concurrency, reports))
}

fn load_agent_batch_tasks(args: &AgentBatch) -> Result<Vec<AgentPoolTask>> {
    let mut tasks = args
        .tasks
        .iter()
        .filter_map(|task| {
            let task = task.trim();
            (!task.is_empty()).then(|| AgentPoolTask::new(task, None))
        })
        .collect::<Vec<_>>();
    if let Some(path) = &args.file {
        let content = fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        tasks.extend(content.lines().filter_map(|line| {
            let task = line.trim();
            (!task.is_empty() && !task.starts_with('#')).then(|| AgentPoolTask::new(task, None))
        }));
    }
    if tasks.is_empty() {
        anyhow::bail!("agent batch requires at least one --task or non-empty --file line");
    }
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_batch_loads_inline_and_file_tasks() {
        let temp = tempfile::tempdir().expect("tempdir");
        let file = temp.path().join("tasks.txt");
        std::fs::write(&file, "\n# comment\ninspect harness\n\ninspect runtime\n").expect("write");
        let tasks = load_agent_batch_tasks(&AgentBatch {
            tasks: vec!["inspect cli".into(), " ".into()],
            file: Some(file),
            profile: None,
            concurrency: 2,
            dry_run: true,
            agent_loop: false,
            loop_max_iterations: 6,
            parent_session: None,
        })
        .expect("tasks");

        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].task, "inspect cli");
        assert_eq!(tasks[1].task, "inspect harness");
        assert_eq!(tasks[2].task, "inspect runtime");
    }
}
