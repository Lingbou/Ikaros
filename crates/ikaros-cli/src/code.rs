// SPDX-License-Identifier: GPL-3.0-only

mod interactive_parse;
mod output;
mod review;
mod rollback;
#[cfg(test)]
mod tests;
mod workflow;

pub(crate) use interactive_parse::parse_interactive_code_command;
pub(crate) use output::print_code_terminal_summary;
pub(crate) use workflow::{
    coding_session_and_registry_for_workflow_with_cancellation, install_coding_cancellation_signal,
};

use crate::{print_approval_hint, print_skill_result, session_and_registry};
use anyhow::{Context, Result};
use clap::Subcommand;
use ikaros_core::IkarosPaths;
use serde_json::json;
use std::path::Path;

#[derive(Debug, Subcommand)]
pub(crate) enum CodeCommand {
    Plan {
        objective: String,
        #[arg(long)]
        diff: Option<String>,
        #[arg(long)]
        model_loop: bool,
        #[arg(long = "max-iterations")]
        max_iterations: Option<usize>,
        #[arg(long = "model-token-budget")]
        model_token_budget: Option<u32>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        turn_id: Option<String>,
    },
    Apply {
        objective: String,
        #[arg(long)]
        diff: String,
        #[arg(long)]
        run_tests: bool,
        #[arg(long)]
        model_loop: bool,
        #[arg(long = "max-iterations")]
        max_iterations: Option<usize>,
        #[arg(long = "model-token-budget")]
        model_token_budget: Option<u32>,
        #[arg(long = "test-command")]
        test_commands: Vec<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        turn_id: Option<String>,
    },
    Test {
        objective: Option<String>,
        #[arg(long = "test-command")]
        test_commands: Vec<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        turn_id: Option<String>,
    },
    Rollback {
        session_id: String,
        #[arg(long)]
        turn_id: String,
        #[arg(long = "rollback-turn-id")]
        rollback_turn_id: Option<String>,
        #[arg(long)]
        run_tests: bool,
        #[arg(long = "test-command")]
        test_commands: Vec<String>,
    },
    GuardedEdit {
        objective: String,
        #[arg(long)]
        diff: Option<String>,
    },
    Iterate {
        objective: Option<String>,
        #[arg(long)]
        diff: Option<String>,
        #[arg(long = "test-analysis-json")]
        test_analysis_json: Option<String>,
    },
    Workflow {
        objective: String,
        #[arg(long)]
        diff: Option<String>,
        #[arg(long, default_value = "plan")]
        mode: String,
        #[arg(long)]
        apply_patch: bool,
        #[arg(long)]
        run_tests: bool,
        #[arg(long)]
        model_loop: bool,
        #[arg(long = "max-iterations")]
        max_iterations: Option<usize>,
        #[arg(long = "model-token-budget")]
        model_token_budget: Option<u32>,
        #[arg(long = "test-command")]
        test_commands: Vec<String>,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        turn_id: Option<String>,
        #[arg(long = "test-analysis-json")]
        test_analysis_json: Option<String>,
    },
    Review {
        #[arg(long)]
        diff: Option<String>,
        #[arg(long = "test-analysis-json")]
        test_analysis_json: Option<String>,
        #[arg(long = "model-notes")]
        model_notes: bool,
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long)]
        turn_id: Option<String>,
    },
}

pub(crate) async fn code_command(
    command: CodeCommand,
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
) -> Result<()> {
    let (session, registry) = session_and_registry(paths, workspace, agent_override)?;
    let (result, model_usage_path) = match command {
        CodeCommand::Plan {
            objective,
            diff,
            model_loop,
            max_iterations,
            model_token_budget,
            session_id,
            turn_id,
        } => {
            workflow::run_coding_workflow_command(
                paths,
                workspace,
                agent_override,
                workflow::CodingWorkflowCommandInput {
                    objective,
                    mode: "plan".into(),
                    diff,
                    apply_patch: false,
                    run_tests: false,
                    model_loop,
                    max_iterations,
                    model_token_budget,
                    test_commands: Vec::new(),
                    session_id,
                    turn_id,
                    test_analysis_json: None,
                },
            )
            .await?
        }
        CodeCommand::Apply {
            objective,
            diff,
            run_tests,
            model_loop,
            max_iterations,
            model_token_budget,
            test_commands,
            session_id,
            turn_id,
        } => {
            workflow::run_coding_workflow_command(
                paths,
                workspace,
                agent_override,
                workflow::CodingWorkflowCommandInput {
                    objective,
                    mode: "edit".into(),
                    diff: Some(diff),
                    apply_patch: true,
                    run_tests,
                    model_loop,
                    max_iterations,
                    model_token_budget,
                    test_commands,
                    session_id,
                    turn_id,
                    test_analysis_json: None,
                },
            )
            .await?
        }
        CodeCommand::Test {
            objective,
            test_commands,
            session_id,
            turn_id,
        } => {
            workflow::run_coding_workflow_command(
                paths,
                workspace,
                agent_override,
                workflow::CodingWorkflowCommandInput {
                    objective: objective.unwrap_or_else(|| "run coding test matrix".into()),
                    mode: "test".into(),
                    diff: None,
                    apply_patch: false,
                    run_tests: true,
                    model_loop: false,
                    max_iterations: None,
                    model_token_budget: None,
                    test_commands,
                    session_id,
                    turn_id,
                    test_analysis_json: None,
                },
            )
            .await?
        }
        CodeCommand::Rollback {
            session_id,
            turn_id,
            rollback_turn_id,
            run_tests,
            test_commands,
        } => {
            let diff = rollback::rollback_diff_for_coding_turn(
                paths,
                workspace,
                agent_override,
                &session_id,
                &turn_id,
            )?;
            workflow::run_coding_workflow_command(
                paths,
                workspace,
                agent_override,
                workflow::CodingWorkflowCommandInput {
                    objective: format!("rollback coding turn {turn_id}"),
                    mode: "edit".into(),
                    diff: Some(diff),
                    apply_patch: true,
                    run_tests,
                    model_loop: false,
                    max_iterations: None,
                    model_token_budget: None,
                    test_commands,
                    session_id: Some(session_id),
                    turn_id: rollback_turn_id.or_else(|| Some(format!("rollback-{turn_id}"))),
                    test_analysis_json: None,
                },
            )
            .await?
        }
        CodeCommand::GuardedEdit { objective, diff } => {
            let mut input = json!({"objective": objective});
            if let Some(diff) = diff {
                input["diff"] = json!(diff);
            }
            (
                session
                    .execute_skill(&registry, "code_edit_guarded", input)
                    .await?,
                None,
            )
        }
        CodeCommand::Iterate {
            objective,
            diff,
            test_analysis_json,
        } => {
            let diff = workflow::resolve_code_diff(&session, &registry, diff).await?;
            let mut input = json!({
                "objective": objective.unwrap_or_else(|| "prepare next guarded patch iteration".into()),
                "diff": diff,
            });
            if let Some(test_analysis_json) = test_analysis_json {
                input["test_analysis"] = serde_json::from_str(&test_analysis_json)
                    .with_context(|| "failed to parse --test-analysis-json")?;
            }
            (
                session
                    .execute_skill(&registry, "code_iterate", input)
                    .await?,
                None,
            )
        }
        CodeCommand::Workflow {
            objective,
            diff,
            mode,
            apply_patch,
            run_tests,
            model_loop,
            max_iterations,
            model_token_budget,
            test_commands,
            session_id,
            turn_id,
            test_analysis_json,
        } => {
            workflow::run_coding_workflow_command(
                paths,
                workspace,
                agent_override,
                workflow::CodingWorkflowCommandInput {
                    objective,
                    mode,
                    diff,
                    apply_patch,
                    run_tests,
                    model_loop,
                    max_iterations,
                    model_token_budget,
                    test_commands,
                    session_id,
                    turn_id,
                    test_analysis_json,
                },
            )
            .await?
        }
        CodeCommand::Review {
            diff,
            test_analysis_json,
            model_notes,
            session_id,
            turn_id,
        } => {
            let diff = workflow::resolve_code_diff(&session, &registry, diff).await?;
            if model_notes {
                let mut input = json!({"diff": diff});
                if let Some(test_analysis_json) = test_analysis_json {
                    input["test_analysis"] = serde_json::from_str(&test_analysis_json)
                        .with_context(|| "failed to parse --test-analysis-json")?;
                }
                let mut result = session
                    .execute_skill(&registry, "code_review", input)
                    .await?;
                let model_usage_path = Some(
                    review::append_model_code_review_notes(&diff, &mut result, paths, &session)
                        .await?,
                );
                (result, model_usage_path)
            } else {
                workflow::run_coding_workflow_command(
                    paths,
                    workspace,
                    agent_override,
                    workflow::CodingWorkflowCommandInput {
                        objective: "review current coding diff".into(),
                        mode: "review".into(),
                        diff: Some(diff),
                        apply_patch: false,
                        run_tests: false,
                        model_loop: false,
                        max_iterations: None,
                        model_token_budget: None,
                        test_commands: Vec::new(),
                        session_id,
                        turn_id,
                        test_analysis_json,
                    },
                )
                .await?
            }
        }
    };
    print_skill_result(&result)?;
    print_code_terminal_summary(&result)?;
    print_approval_hint(&result);
    println!("audit: {}", session.audit.path().display());
    if let Some(path) = model_usage_path {
        println!("model_usage: {}", path.display());
    }
    if let Some(log) = session.approvals.log() {
        println!("approvals: {}", log.path().display());
    }
    Ok(())
}
