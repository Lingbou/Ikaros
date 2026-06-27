// SPDX-License-Identifier: GPL-3.0-only

use crate::{
    print_approval_hint, print_skill_result, resolve_agent_instance, session_and_registry,
    session_and_registry_for_instance, skill_env,
};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use ikaros_core::{IkarosPaths, ToolResult, redact_json, redact_secrets};
use ikaros_harness::{AuditEvent, CancellationToken, ExecutionSession, SkillRegistry};
use ikaros_models::{
    ModelMessage, ModelRequest, ModelRequestDiagnostic, ModelRequestOptions, ModelUsageLedger,
    governed_provider_from_config_with_http_client,
};
use ikaros_runtime::EgressModelHttpClient;
use ikaros_session::{
    AgentEvent, AgentEventKind, AgentEventSource, ApprovalRecord as SessionApprovalRecord,
    ApprovalStatus as SessionApprovalStatus, SessionId, SessionSource, SessionStore,
    SqliteSessionStore, TurnId,
};
use ikaros_skills::{CodingSessionConfig, builtin_registry};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

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

#[derive(Debug, Parser)]
#[command(name = "code")]
struct InteractiveCodeCli {
    #[command(subcommand)]
    command: CodeCommand,
}

pub(crate) fn parse_interactive_code_command(input: &str) -> Result<CodeCommand> {
    let args = split_interactive_code_line(input)?;
    let cli = InteractiveCodeCli::try_parse_from(
        std::iter::once("code").chain(args.iter().map(String::as_str)),
    )?;
    Ok(cli.command)
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
            run_coding_workflow_command(
                paths,
                workspace,
                agent_override,
                CodingWorkflowCommandInput {
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
            run_coding_workflow_command(
                paths,
                workspace,
                agent_override,
                CodingWorkflowCommandInput {
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
            run_coding_workflow_command(
                paths,
                workspace,
                agent_override,
                CodingWorkflowCommandInput {
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
            let diff = rollback_diff_for_coding_turn(
                paths,
                workspace,
                agent_override,
                &session_id,
                &turn_id,
            )?;
            run_coding_workflow_command(
                paths,
                workspace,
                agent_override,
                CodingWorkflowCommandInput {
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
            let diff = resolve_code_diff(&session, &registry, diff).await?;
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
            run_coding_workflow_command(
                paths,
                workspace,
                agent_override,
                CodingWorkflowCommandInput {
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
            let diff = resolve_code_diff(&session, &registry, diff).await?;
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
                    append_model_code_review_notes(&diff, &mut result, paths, &session).await?,
                );
                (result, model_usage_path)
            } else {
                run_coding_workflow_command(
                    paths,
                    workspace,
                    agent_override,
                    CodingWorkflowCommandInput {
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

fn split_interactive_code_line(input: &str) -> Result<Vec<String>> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in input.chars() {
        if escaped {
            current.push(decode_interactive_code_escape(ch));
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        match ch {
            '"' | '\'' => quote = Some(ch),
            ch if ch.is_whitespace() => {
                if !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
            }
            _ => current.push(ch),
        }
    }
    if escaped {
        anyhow::bail!("unterminated escape in /code command");
    }
    if quote.is_some() {
        anyhow::bail!("unterminated quote in /code command");
    }
    if !current.is_empty() {
        args.push(current);
    }
    Ok(args)
}

fn decode_interactive_code_escape(ch: char) -> char {
    match ch {
        'n' => '\n',
        'r' => '\r',
        't' => '\t',
        other => other,
    }
}

struct CodingWorkflowCommandInput {
    objective: String,
    mode: String,
    diff: Option<String>,
    apply_patch: bool,
    run_tests: bool,
    model_loop: bool,
    max_iterations: Option<usize>,
    model_token_budget: Option<u32>,
    test_commands: Vec<String>,
    session_id: Option<String>,
    turn_id: Option<String>,
    test_analysis_json: Option<String>,
}

async fn run_coding_workflow_command(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    command: CodingWorkflowCommandInput,
) -> Result<(ToolResult, Option<PathBuf>)> {
    let cancellation = CancellationToken::new();
    if command.model_loop {
        install_coding_cancellation_signal(cancellation.clone());
    }
    let (session, registry, session_id, turn_id) =
        coding_session_and_registry_for_workflow_with_cancellation(
            paths,
            workspace,
            agent_override,
            command.session_id,
            command.turn_id,
            command.model_loop,
            cancellation,
        )?;
    let mut input = json!({
        "objective": command.objective,
        "mode": command.mode,
        "apply_patch": command.apply_patch,
        "run_tests": command.run_tests,
        "model_loop": command.model_loop,
        "session_id": session_id.as_str(),
        "turn_id": turn_id.as_str(),
    });
    if let Some(max_iterations) = command.max_iterations {
        input["max_iterations"] = json!(max_iterations);
    }
    if let Some(model_token_budget) = command.model_token_budget {
        input["model_token_budget"] = json!(model_token_budget);
    }
    if let Some(diff) = command.diff {
        input["diff"] = json!(diff);
    }
    if !command.test_commands.is_empty() {
        input["test_commands"] = json!(command.test_commands);
    }
    if let Some(test_analysis_json) = command.test_analysis_json {
        input["test_analysis"] = serde_json::from_str(&test_analysis_json)
            .with_context(|| "failed to parse --test-analysis-json")?;
    }
    let result = session
        .execute_skill(&registry, "code_workflow", input)
        .await?;
    record_coding_workflow_approval_request(
        paths,
        workspace,
        agent_override,
        &session,
        &result,
        &session_id,
        &turn_id,
    )?;
    Ok((result, None))
}

fn record_coding_workflow_approval_request(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session: &ExecutionSession,
    result: &ToolResult,
    session_id: &SessionId,
    turn_id: &TurnId,
) -> Result<()> {
    if result
        .output
        .get("decision")
        .and_then(serde_json::Value::as_str)
        != Some("ask_user")
    {
        return Ok(());
    }
    let Some(approval_id) = result
        .output
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(());
    };
    let Some(record) = session.approvals.get(approval_id)? else {
        return Ok(());
    };
    let config = ikaros_core::IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let store = SqliteSessionStore::new(&agent.state_dir);
    store.append_approval(&SessionApprovalRecord {
        approval_id: approval_id.into(),
        session_id: session_id.clone(),
        turn_id: Some(turn_id.clone()),
        at: time::OffsetDateTime::now_utc(),
        status: SessionApprovalStatus::Requested,
        request: redact_json(serde_json::to_value(&record.request)?),
        decision: None,
    })?;
    store.append_agent_event(&AgentEvent::new(
        session_id.clone(),
        turn_id.clone(),
        None,
        AgentEventSource::Harness,
        AgentEventKind::ApprovalRequested,
        json!({
            "approval_id": approval_id,
            "tool": &record.request.call.name,
            "risk": format!("{:?}", record.request.call.risk),
        }),
    ))?;
    Ok(())
}

pub(crate) fn coding_session_and_registry_for_workflow_with_cancellation(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: Option<String>,
    turn_id: Option<String>,
    include_model_provider: bool,
    cancellation: CancellationToken,
) -> Result<(ExecutionSession, SkillRegistry, SessionId, TurnId)> {
    let config = ikaros_core::IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let (session, _) = session_and_registry_for_instance(paths, &config, &agent)?;
    let session_id = session_id.map(SessionId::from).unwrap_or_default();
    let turn_id = turn_id.map(TurnId::from).unwrap_or_default();
    let mut env = skill_env(paths, &agent.workspace, &config)?;
    let coding_model_provider = if include_model_provider {
        let model_config = agent.model_config(&config.model.default);
        let model_provider =
            agent.effective_model_provider_config(&config.model.default, &config.providers.model);
        Some(Arc::from(governed_provider_from_config_with_http_client(
            model_config,
            &model_provider,
            &paths.audit_dir,
            Some(Arc::new(EgressModelHttpClient::new(session.env.clone()))),
        )?))
    } else {
        None
    };
    env.coding_session = Some(CodingSessionConfig {
        store: Arc::new(SqliteSessionStore::new(&agent.state_dir)),
        session_id: session_id.clone(),
        turn_id: turn_id.clone(),
        source: SessionSource::Cli,
        agent_id: Some(agent.agent_id.clone()),
        workspace: Some(agent.workspace.clone()),
        model_provider: coding_model_provider,
        cancellation,
    });
    Ok((session, builtin_registry(env), session_id, turn_id))
}

pub(crate) fn install_coding_cancellation_signal(cancellation: CancellationToken) {
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            cancellation.cancel();
            eprintln!(
                "coding_cancel_requested: waiting for the running provider/tool step to stop"
            );
        }
    });
}

pub(crate) fn print_code_terminal_summary(result: &ToolResult) -> Result<()> {
    if let Some(context) = result.output.get("approval_context") {
        print_code_approval_context(context);
    }
    if result
        .output
        .get("events")
        .and_then(serde_json::Value::as_array)
        .is_some()
    {
        print_coding_progress(&result.output);
    }
    Ok(())
}

fn print_code_approval_context(context: &serde_json::Value) {
    let operations = &context["operations"];
    println!("approval_scope:");
    println!(
        "  provider_call: {}",
        operations["provider_call"].as_bool().unwrap_or(false)
    );
    println!(
        "  workspace_write: {}",
        operations["workspace_write"].as_bool().unwrap_or(false)
    );
    let shell_requested = operations["shell"].as_bool().unwrap_or(false);
    println!("  shell: {shell_requested}");
    let shell_commands = operations["shell_commands"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if shell_commands.is_empty() {
        if shell_requested
            && operations["shell_commands_inferred"]
                .as_bool()
                .unwrap_or(false)
        {
            println!("  shell_commands: inferred from workspace");
        } else {
            println!("  shell_commands: none");
        }
    } else {
        println!("  shell_commands:");
        for command in shell_commands {
            let command_text = command["command"].as_str().unwrap_or("<unknown>");
            let reason = command["reason"].as_str().unwrap_or("unspecified");
            println!(
                "    - {} ({})",
                redact_secrets(command_text),
                redact_secrets(reason)
            );
        }
    }
    println!(
        "  provider: {}",
        context["provider"]["name"]
            .as_str()
            .unwrap_or("not_configured")
    );
    println!(
        "  session: {} turn={}",
        context["session"]["session_id"]
            .as_str()
            .unwrap_or("<generated>"),
        context["session"]["turn_id"]
            .as_str()
            .unwrap_or("<generated>")
    );
}

fn print_coding_progress(output: &serde_json::Value) {
    let Some(events) = output.get("events").and_then(serde_json::Value::as_array) else {
        return;
    };
    println!("coding_progress:");
    for event in events {
        let kind = event["kind"].as_str().unwrap_or("<unknown>");
        let summary = event["summary"].as_str().unwrap_or_default();
        println!("  - {}: {}", kind, redact_secrets(summary));
    }
    if let Some(loop_report) = output.get("loop_report") {
        println!(
            "coding_result: status={} iterations={} reason={}",
            loop_report["status"].as_str().unwrap_or("<unknown>"),
            loop_report["iterations"]
                .as_u64()
                .map(|value| value.to_string())
                .unwrap_or_else(|| "0".into()),
            loop_report["reason"]
                .as_str()
                .map(redact_secrets)
                .unwrap_or_default()
        );
    }
}

fn rollback_diff_for_coding_turn(
    paths: &IkarosPaths,
    workspace: &Path,
    agent_override: Option<&str>,
    session_id: &str,
    turn_id: &str,
) -> Result<String> {
    let config = ikaros_core::IkarosConfig::load(&paths.config)?;
    let agent = resolve_agent_instance(&config, agent_override, workspace, &paths.home)?;
    let store = SqliteSessionStore::new(&agent.state_dir);
    let session_id = SessionId::from(session_id.to_owned());
    let turn_id = TurnId::from(turn_id.to_owned());
    let replay = store
        .replay_session(&session_id)?
        .with_context(|| format!("coding session not found: {}", session_id.as_str()))?;
    let diff = replay
        .agent_events
        .iter()
        .filter(|event| event.turn_id == turn_id)
        .filter(|event| matches!(event.kind, AgentEventKind::CodingTurn))
        .filter(|event| event.payload["kind"] == "diff_updated")
        .filter_map(|event| event.payload["payload"]["unified_diff"].as_str())
        .next_back()
        .with_context(|| {
            format!(
                "coding turn {} has no rollbackable diff_updated event",
                turn_id.as_str()
            )
        })?;
    reverse_unified_diff(diff)
}

fn reverse_unified_diff(diff: &str) -> Result<String> {
    let mut reversed = Vec::new();
    let mut pending_source_header: Option<String> = None;
    let mut change_group = Vec::<String>::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            let mut parts = rest.split_whitespace();
            let Some(left) = parts.next() else {
                anyhow::bail!("invalid diff --git header");
            };
            let Some(right) = parts.next() else {
                anyhow::bail!("invalid diff --git header");
            };
            reversed.push(format!("diff --git {right} {left}"));
            continue;
        }
        if let Some(path) = line.strip_prefix("rename from ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            reversed.push(format!("rename to {path}"));
            continue;
        }
        if let Some(path) = line.strip_prefix("rename to ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            reversed.push(format!("rename from {path}"));
            continue;
        }
        if line.starts_with("--- ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            pending_source_header = Some(line.replacen("--- ", "+++ ", 1));
            continue;
        }
        if line.starts_with("+++ ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            reversed.push(line.replacen("+++ ", "--- ", 1));
            if let Some(source) = pending_source_header.take() {
                reversed.push(source);
            }
            continue;
        }
        if line.starts_with("@@ ") {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            reversed.push(reverse_hunk_header(line)?);
            continue;
        }
        if line.starts_with('+') || line.starts_with('-') {
            change_group.push(line.to_owned());
        } else {
            flush_reversed_change_group(&mut reversed, &mut change_group);
            reversed.push(line.to_owned());
        }
    }
    flush_reversed_change_group(&mut reversed, &mut change_group);
    if pending_source_header.is_some() {
        anyhow::bail!("invalid unified diff: missing +++ header");
    }
    Ok(format!("{}\n", reversed.join("\n")))
}

fn flush_reversed_change_group(output: &mut Vec<String>, group: &mut Vec<String>) {
    if group.is_empty() {
        return;
    }
    let mut removed = Vec::new();
    let mut added = Vec::new();
    for line in group.drain(..) {
        if let Some(rest) = line.strip_prefix('+') {
            added.push(rest.to_owned());
        } else if let Some(rest) = line.strip_prefix('-') {
            removed.push(rest.to_owned());
        }
    }
    output.extend(added.into_iter().map(|line| format!("-{line}")));
    output.extend(removed.into_iter().map(|line| format!("+{line}")));
}

fn reverse_hunk_header(line: &str) -> Result<String> {
    let Some(rest) = line.strip_prefix("@@ ") else {
        anyhow::bail!("invalid hunk header");
    };
    let Some((ranges, suffix)) = rest.split_once(" @@") else {
        anyhow::bail!("invalid hunk header");
    };
    let mut parts = ranges.split_whitespace();
    let Some(old_range) = parts.next() else {
        anyhow::bail!("invalid hunk header");
    };
    let Some(new_range) = parts.next() else {
        anyhow::bail!("invalid hunk header");
    };
    if parts.next().is_some() || !old_range.starts_with('-') || !new_range.starts_with('+') {
        anyhow::bail!("invalid hunk header ranges");
    }
    Ok(format!(
        "@@ -{} +{} @@{}",
        &new_range[1..],
        &old_range[1..],
        suffix
    ))
}

async fn resolve_code_diff(
    session: &ExecutionSession,
    registry: &SkillRegistry,
    diff: Option<String>,
) -> Result<String> {
    if let Some(diff) = diff {
        return Ok(diff);
    }
    let diff_result = session
        .execute_skill(registry, "git_diff", json!({"stat": false}))
        .await?;
    if !diff_result.ok {
        anyhow::bail!(
            "failed to collect current git diff: {}",
            diff_result.summary
        );
    }
    Ok(diff_result
        .output
        .get("stdout")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string())
}

async fn append_model_code_review_notes(
    diff: &str,
    result: &mut ToolResult,
    paths: &IkarosPaths,
    session: &ExecutionSession,
) -> Result<PathBuf> {
    let config = ikaros_core::IkarosConfig::load(&paths.config)?;
    let agent_override = session
        .sandbox
        .agent
        .as_ref()
        .and_then(|agent| agent.agent_id.as_deref())
        .or_else(|| {
            session
                .sandbox
                .agent
                .as_ref()
                .map(|agent| agent.profile_name.as_str())
        });
    let agent = resolve_agent_instance(
        &config,
        agent_override,
        &session.sandbox.workspace_root,
        &paths.home,
    )?;
    let model_config = agent.model_config(&config.model.default);
    let model_provider =
        agent.effective_model_provider_config(&config.model.default, &config.providers.model);
    let provider = governed_provider_from_config_with_http_client(
        model_config,
        &model_provider,
        &paths.audit_dir,
        Some(Arc::new(EgressModelHttpClient::new(session.env.clone()))),
    )?;
    let usage_ledger = ModelUsageLedger::new(&paths.audit_dir);
    let prompt = build_model_code_review_prompt(diff, &result.output)?;
    let response = provider
        .generate(ModelRequest {
            messages: vec![
                ModelMessage::system(
                    "You are the Ikaros code review assistant. Use the heuristic review and redacted diff excerpt to produce concise review notes. Include residual risks, focused tests, and a guarded patch iteration plan. Do not reproduce the full diff, reveal secrets, request commits, bypass approvals, or suggest writing outside the workspace.",
                ),
                ModelMessage::user(prompt.clone()),
            ],
            options: ModelRequestOptions {
                max_tokens: Some(700),
                temperature: Some(0.2),
                ..ModelRequestOptions::default()
            },
            tools: Vec::new(),
        })
        .await?;
    let diagnostics = response
        .diagnostics
        .iter()
        .cloned()
        .map(ModelRequestDiagnostic::sanitized)
        .collect::<Vec<_>>();
    session.audit.append(AuditEvent::new(
        "code_model_review_result",
        None,
        "model-assisted code review generated",
        json!({
            "provider": response.provider,
            "model": response.model,
            "usage": response.usage,
            "diagnostics": diagnostics.clone(),
            "prompt_chars": prompt.chars().count(),
        }),
    )?)?;
    let notes = json!({
        "provider": response.provider,
        "model": response.model,
        "content": redact_secrets(&response.content),
        "usage": response.usage,
        "diagnostics": diagnostics,
        "prompt_chars": prompt.chars().count(),
    });
    if let Some(output) = result.output.as_object_mut() {
        output.insert("model_notes".into(), notes);
    } else {
        let original = std::mem::take(&mut result.output);
        result.output = json!({"review": original, "model_notes": notes});
    }
    result.summary = format!("{} with model notes", result.summary);
    Ok(usage_ledger.path().to_path_buf())
}

fn build_model_code_review_prompt(diff: &str, review_output: &serde_json::Value) -> Result<String> {
    let review_json = serde_json::to_string_pretty(review_output)?;
    Ok(redact_secrets(&format!(
        "Heuristic review report:\n{}\n\nRedacted diff excerpt:\n{}\n\nReturn concise notes with these headings: Residual Risks, Focused Tests, Guarded Patch Iteration. Keep recommendations local-first and approval-aware.",
        bounded_redacted_text(&review_json, 8000),
        bounded_redacted_text(diff, 12000),
    )))
}

fn bounded_redacted_text(text: &str, max_chars: usize) -> String {
    let redacted = redact_secrets(text);
    let mut chars = redacted.chars();
    let mut output = chars.by_ref().take(max_chars.max(1)).collect::<String>();
    if chars.next().is_some() {
        output.push_str("\n[TRUNCATED]");
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_code_review_prompt_redacts_and_truncates() {
        let diff = format!(
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1,2 @@\n+token=abc123\n+{}\n",
            "x".repeat(13_000)
        );
        let review = json!({
            "summary": "review saw token=abc123",
            "findings": [{"title": "secret-like addition"}],
        });
        let prompt = build_model_code_review_prompt(&diff, &review).expect("prompt");
        assert!(prompt.contains("Heuristic review report"));
        assert!(prompt.contains("Guarded Patch Iteration"));
        assert!(prompt.contains("[REDACTED_SECRET]"));
        assert!(prompt.contains("[TRUNCATED]"));
        assert!(!prompt.contains("abc123"));
    }

    #[test]
    fn reverse_unified_diff_swaps_headers_hunks_and_lines() {
        let diff = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-old
+new
";
        let reversed = reverse_unified_diff(diff).expect("reverse diff");
        assert!(reversed.contains("diff --git b/src/lib.rs a/src/lib.rs"));
        assert!(reversed.contains("--- b/src/lib.rs\n+++ a/src/lib.rs"));
        assert!(reversed.contains("@@ -1 +1 @@"));
        assert!(reversed.contains("-new\n+old"));
    }

    #[test]
    fn interactive_code_parser_supports_quoted_objective() {
        let command = parse_interactive_code_command(
            r#"plan "prepare quoted objective" --session-id chat-code-session --turn-id chat-code-turn"#,
        )
        .expect("parse /code command");
        match command {
            CodeCommand::Plan {
                objective,
                session_id,
                turn_id,
                ..
            } => {
                assert_eq!(objective, "prepare quoted objective");
                assert_eq!(session_id.as_deref(), Some("chat-code-session"));
                assert_eq!(turn_id.as_deref(), Some("chat-code-turn"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn interactive_code_parser_decodes_escaped_newlines_in_quoted_diff() {
        let command = parse_interactive_code_command(
            r#"apply "apply escaped diff" --diff "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-old\n+new\n""#,
        )
        .expect("parse /code apply");
        match command {
            CodeCommand::Apply {
                objective, diff, ..
            } => {
                assert_eq!(objective, "apply escaped diff");
                assert!(diff.contains("diff --git a/src/lib.rs b/src/lib.rs\n"));
                assert!(diff.contains("-old\n+new\n"));
                assert!(!diff.contains("\\n"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn interactive_code_parser_rejects_unterminated_quote() {
        let error = parse_interactive_code_command(r#"plan "missing end"#)
            .expect_err("unterminated quote should fail");
        assert!(error.to_string().contains("unterminated quote"));
    }
}
