// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use clap::Subcommand;
use ikaros_body::{BodyAdapter, BodyStatus, CliBodyAdapter};
use ikaros_core::{ContextBuilder, IkarosPaths};
use ikaros_harness::{PlanStepStatus, TaskExecutionReport};
use ikaros_runtime::{TaskRunOptions, execute_task_text_with_options};
use ikaros_soul::{EmotionState, RuntimeSignal, load_or_default};
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum TaskCommand {
    Run {
        task: String,
        #[arg(long)]
        dry_run: bool,
        #[arg(long)]
        agent_loop: bool,
        #[arg(long, default_value_t = 6)]
        loop_max_iterations: u32,
    },
}

pub(crate) async fn task_command(
    command: TaskCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    match command {
        TaskCommand::Run {
            task,
            dry_run,
            agent_loop,
            loop_max_iterations,
        } => {
            run_task(
                task,
                TaskRunOptions {
                    dry_run,
                    agent_loop,
                    loop_max_iterations,
                    ..TaskRunOptions::default()
                },
                paths,
                workspace,
                agent_override,
            )
            .await?
        }
    }
    Ok(())
}

async fn run_task(
    task_text: String,
    options: TaskRunOptions,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    paths.ensure()?;
    let persona = load_or_default(&paths.persona)?;
    let run = execute_task_text_with_options(
        task_text,
        options.clone(),
        paths,
        workspace,
        agent_override,
    )
    .await?;
    let task = run.task.clone();
    let plan = run.plan.clone();
    let execution_report = run.report.clone();
    let memory_summary = step_summary(&execution_report, "memory_search", "memory search skipped");
    let rag_summary = step_summary(&execution_report, "rag_search", "RAG search skipped");
    let summary_text = step_summary(&execution_report, "task_summarize", "summary skipped");
    let task_memory_summary = match step_status(&execution_report, "memory_append") {
        Some(PlanStepStatus::Succeeded) => {
            step_summary(&execution_report, "memory_append", "task memory written")
        }
        Some(status) => format!(
            "task memory skipped ({status:?}): {}",
            step_summary(&execution_report, "memory_append", "not executed")
        ),
        None => "task memory skipped: step not present".into(),
    };
    let context = ContextBuilder::new()
        .task(task.clone())
        .persona_context(persona.context_summary())
        .retrieved_memory_context(vec![memory_summary.clone(), task_memory_summary.clone()])
        .rag_context(vec![rag_summary.clone()])
        .build();
    let mut body_status = BodyStatus::new(
        persona.identity.name.clone(),
        format!("{:?}", run.final_emotion),
    )
    .with_task(task.id.clone(), Some(task.state.clone()))
    .with_context_sources(
        vec![memory_summary.clone(), task_memory_summary.clone()],
        vec![rag_summary.clone()],
    )
    .with_policy_decisions(run.policy_decisions.clone())
    .with_audit_path(&run.audit_path);
    if let Some(path) = &run.approvals_path {
        body_status = body_status.with_approvals_path(path);
    }

    println!("{}", CliBodyAdapter.render_status(&body_status));
    println!("persona: {}", persona.identity.name);
    println!(
        "emotion: {:?} -> {:?}",
        EmotionState::for_runtime_signal(RuntimeSignal::Planning),
        run.final_emotion
    );
    println!("task_id: {}", task.id);
    println!("dry_run: {}", options.dry_run);
    println!("agent_loop: {}", options.agent_loop);
    println!("state: {:?}", task.state);
    println!("plan:");
    for step in &plan.steps {
        println!("- [{:?}] {}", step.risk, step.description);
    }
    println!("steps:");
    for step in &execution_report.steps {
        println!(
            "- [{:?}] {} ({} attempt(s)): {}",
            step.status, step.skill, step.attempts, step.summary
        );
        if let Some(approval_id) = &step.approval_id {
            println!("  approval: {approval_id}");
            println!("  next: ikaros approval approve {approval_id}");
        }
    }
    println!("context_persona_chars: {}", context.persona_context.len());
    println!("memory: {}", memory_summary);
    println!("memory_write: {}", task_memory_summary);
    println!("rag: {}", rag_summary);
    println!("summary: {}", summary_text);
    if let Some(loop_report) = &run.agent_loop {
        println!(
            "loop: stop={:?} iterations={} tools={} provider={} model={}",
            loop_report.stop_reason,
            loop_report.iterations,
            loop_report.tool_results.len(),
            loop_report.provider,
            loop_report.model
        );
    }
    println!("audit: {}", run.audit_path.display());
    println!("{}", serde_json::to_string_pretty(&execution_report)?);
    Ok(())
}

fn step_summary(report: &TaskExecutionReport, skill: &str, fallback: &str) -> String {
    report
        .steps
        .iter()
        .find(|step| step.skill == skill)
        .filter(|step| !step.summary.is_empty())
        .map(|step| step.summary.clone())
        .unwrap_or_else(|| fallback.into())
}

fn step_status(report: &TaskExecutionReport, skill: &str) -> Option<PlanStepStatus> {
    report
        .steps
        .iter()
        .find(|step| step.skill == skill)
        .map(|step| step.status.clone())
}
