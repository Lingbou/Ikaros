// SPDX-License-Identifier: GPL-3.0-only

use super::{
    handoff::run_agent_handoff_with_options,
    report::pool_item_from_result,
    types::{AgentPoolItemReport, AgentPoolReport, AgentPoolTask},
};
use crate::task::TaskRunOptions;
use ikaros_core::{IkarosError, IkarosPaths, Result, redact_secrets};
use std::path::Path;

pub async fn run_agent_pool(
    paths: &IkarosPaths,
    workspace: &Path,
    tasks: Vec<AgentPoolTask>,
    default_profile: Option<&str>,
    dry_run: bool,
    concurrency: usize,
) -> Result<AgentPoolReport> {
    run_agent_pool_with_options(
        paths,
        workspace,
        tasks,
        default_profile,
        TaskRunOptions::deterministic(dry_run),
        concurrency,
    )
    .await
}

pub async fn run_agent_pool_with_options(
    paths: &IkarosPaths,
    workspace: &Path,
    tasks: Vec<AgentPoolTask>,
    default_profile: Option<&str>,
    options: TaskRunOptions,
    concurrency: usize,
) -> Result<AgentPoolReport> {
    if tasks.is_empty() {
        return Err(IkarosError::Message(
            "agent pool requires at least one task".into(),
        ));
    }
    if concurrency == 0 {
        return Err(IkarosError::Message(
            "agent pool concurrency must be greater than zero".into(),
        ));
    }
    let concurrency = concurrency.min(tasks.len());
    let mut reports = Vec::new();
    let indexed_tasks = tasks.into_iter().enumerate().collect::<Vec<_>>();
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
                let result = run_agent_handoff_with_options(
                    &paths,
                    &workspace,
                    profile.as_deref(),
                    task_text.clone(),
                    options,
                )
                .await;
                pool_item_from_result(index, task_text, profile, result)
            });
            handles.push((index, task.task, requested_profile, handle));
        }
        for (index, task_text, profile, handle) in handles {
            match handle.await {
                Ok(report) => reports.push(report),
                Err(error) => reports.push(AgentPoolItemReport {
                    index,
                    task: redact_secrets(&task_text),
                    profile,
                    ok: false,
                    state: None,
                    report: None,
                    error: Some(redact_secrets(&format!(
                        "agent worker task join failed: {error}"
                    ))),
                }),
            }
        }
    }
    reports.sort_by_key(|report| report.index);
    let succeeded = reports.iter().filter(|report| report.ok).count();
    let total = reports.len();
    Ok(AgentPoolReport {
        dry_run: options.dry_run,
        agent_loop: options.agent_loop,
        concurrency,
        total,
        succeeded,
        failed: total.saturating_sub(succeeded),
        reports,
    })
}
