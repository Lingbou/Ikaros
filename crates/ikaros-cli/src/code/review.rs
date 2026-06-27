// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_core::{IkarosPaths, ToolResult, redact_secrets};
use ikaros_execution::harness::{AuditEvent, ExecutionSession};
use ikaros_host::{chat_model_services_for_session, host_agent_context};
use ikaros_providers::model::{
    ModelMessage, ModelRequest, ModelRequestDiagnostic, ModelRequestOptions, ModelUsageLedger,
};
use serde_json::json;
use std::path::PathBuf;

pub(super) async fn append_model_code_review_notes(
    diff: &str,
    result: &mut ToolResult,
    paths: &IkarosPaths,
    session: &ExecutionSession,
) -> Result<PathBuf> {
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
    let host = host_agent_context(paths, &session.sandbox.workspace_root, agent_override)?;
    let provider =
        chat_model_services_for_session(paths, &host.config, &host.agent_instance, session)?
            .provider;
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

pub(super) fn build_model_code_review_prompt(
    diff: &str,
    review_output: &serde_json::Value,
) -> Result<String> {
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
