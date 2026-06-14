// SPDX-License-Identifier: GPL-3.0-only

use crate::{print_approval_hint, print_skill_result, session_and_registry};
use anyhow::{Context, Result};
use clap::Subcommand;
use ikaros_core::{IkarosPaths, ToolResult, redact_secrets};
use ikaros_harness::{AuditEvent, ExecutionSession, SkillRegistry};
use ikaros_models::{ModelMessage, ModelRequest, ModelUsageLedger, governed_provider_from_config};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Debug, Subcommand)]
pub(crate) enum CodeCommand {
    Plan {
        objective: String,
        #[arg(long)]
        diff: Option<String>,
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
    Review {
        #[arg(long)]
        diff: Option<String>,
        #[arg(long = "test-analysis-json")]
        test_analysis_json: Option<String>,
        #[arg(long = "model-notes")]
        model_notes: bool,
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
        CodeCommand::Plan { objective, diff } => {
            let mut input = json!({"objective": objective, "plan_only": true});
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
        CodeCommand::Review {
            diff,
            test_analysis_json,
            model_notes,
        } => {
            let diff = resolve_code_diff(&session, &registry, diff).await?;
            let mut input = json!({"diff": diff});
            if let Some(test_analysis_json) = test_analysis_json {
                input["test_analysis"] = serde_json::from_str(&test_analysis_json)
                    .with_context(|| "failed to parse --test-analysis-json")?;
            }
            let mut result = session
                .execute_skill(&registry, "code_review", input)
                .await?;
            let model_usage_path = if model_notes {
                Some(append_model_code_review_notes(&diff, &mut result, paths, &session).await?)
            } else {
                None
            };
            (result, model_usage_path)
        }
    };
    print_skill_result(&result)?;
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
    let provider = governed_provider_from_config(
        &config.model.default,
        &config.providers.model,
        &paths.audit_dir,
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
            max_tokens: Some(700),
            temperature: Some(0.2),
            tools: Vec::new(),
        })
        .await?;
    session.audit.append(AuditEvent::new(
        "code_model_review_result",
        None,
        "model-assisted code review generated",
        json!({
            "provider": response.provider,
            "model": response.model,
            "usage": response.usage,
            "prompt_chars": prompt.chars().count(),
        }),
    )?)?;
    let notes = json!({
        "provider": response.provider,
        "model": response.model,
        "content": redact_secrets(&response.content),
        "usage": response.usage,
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
}
