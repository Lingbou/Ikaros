// SPDX-License-Identifier: GPL-3.0-only

use crate::resolve_agent;
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use ikaros_core::{AgentProfile, IkarosConfig, IkarosPaths, redact_json};
use ikaros_runtime::{
    AgentPoolTask, TaskRunOptions, run_agent_handoff_with_options, run_agent_pool_with_options,
};
use serde_json::{Map, Value, json};
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
    let config = IkarosConfig::load(&paths.config)?;
    match command {
        AgentCommand::List => {
            println!("{}", serde_json::to_string_pretty(&agent_list(&config))?);
        }
        AgentCommand::Show { profile } => {
            let requested = profile.as_deref().or(agent_override);
            let agent = resolve_agent(&config, requested)?;
            println!(
                "{}",
                serde_json::to_string_pretty(&agent_show(&agent.name, &agent.profile))?
            );
        }
        AgentCommand::Run(args) => {
            let requested = args.profile.as_deref().or(agent_override);
            let agent = resolve_agent(&config, requested)?;
            let report = run_agent_handoff_with_options(
                paths,
                workspace,
                Some(&agent.name),
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
            let requested = args.profile.as_deref().or(agent_override);
            let profile = match requested {
                Some(profile) => Some(resolve_agent(&config, Some(profile))?.name),
                None => None,
            };
            let tasks = load_agent_batch_tasks(&args)?;
            let report = run_agent_pool_with_options(
                paths,
                workspace,
                tasks,
                profile.as_deref(),
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

fn agent_list(config: &IkarosConfig) -> Value {
    let profiles = config
        .agent
        .profiles
        .iter()
        .map(|(name, profile)| {
            (
                name.clone(),
                json!({
                    "mode": profile.mode,
                    "description": profile.description,
                    "memory_context": profile.memory_context,
                    "rag_context": profile.rag_context,
                    "permissions": permissions_json(profile),
                }),
            )
        })
        .collect::<Map<_, _>>();
    redact_json(json!({
        "default": config.agent.default,
        "profiles": profiles,
    }))
}

fn agent_show(name: &str, profile: &AgentProfile) -> Value {
    redact_json(json!({
        "name": name,
        "mode": profile.mode,
        "description": profile.description,
        "persona_overlay": profile.persona_overlay,
        "memory_context": profile.memory_context,
        "rag_context": profile.rag_context,
        "permissions": permissions_json(profile),
    }))
}

fn permissions_json(profile: &AgentProfile) -> Value {
    json!({
        "workspace_writes": profile.workspace_writes,
        "shell": profile.shell,
        "network": profile.network,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ikaros_core::AgentPermission;

    #[test]
    fn agent_show_redacts_secret_like_profile_text() {
        let mut profile = AgentProfile::general();
        profile.description = "use token=abc123 for nothing".into();
        profile.persona_overlay = "never echo sk-test-secret".into();

        let rendered = serde_json::to_string(&agent_show("safe", &profile)).expect("json");

        assert!(!rendered.contains("abc123"));
        assert!(!rendered.contains("sk-test-secret"));
        assert!(rendered.contains("[REDACTED_SECRET]"));
    }

    #[test]
    fn permissions_snapshot_preserves_policy_intent() {
        let mut profile = AgentProfile::plan();
        profile.workspace_writes = AgentPermission::Deny;
        profile.shell = AgentPermission::Ask;
        profile.network = AgentPermission::Allow;

        let rendered = permissions_json(&profile);

        assert_eq!(rendered["workspace_writes"], "deny");
        assert_eq!(rendered["shell"], "ask");
        assert_eq!(rendered["network"], "allow");
    }

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
