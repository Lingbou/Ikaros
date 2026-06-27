// SPDX-License-Identifier: GPL-3.0-only

use super::{interactive::InteractiveChatRuntime, terminal};
use ikaros_core::redact_secrets;
use serde_json::json;

pub(in crate::chat) fn print_interactive_command_error(
    runtime: &InteractiveChatRuntime,
    input: &str,
    error: &anyhow::Error,
) {
    if runtime.default_inline_stdout() {
        let _ = terminal::print_inline_history_lines(&[format!(
            "• Command failed. {}",
            human_error_summary_for_stdout(&redact_secrets(&error.to_string()))
        )]);
        return;
    }
    if runtime.fullscreen_stdout_quiet() {
        return;
    }
    let command = input.split_whitespace().next().unwrap_or(input);
    let message = error.to_string();
    println!(
        "interactive_command: failed command={} error={}",
        redact_secrets(command),
        redact_secrets(&message)
    );
    println!("{}", interactive_command_error_json_line(input, error));
    println!(
        "interactive_command_recovery_hint: use /help or /commands to inspect available commands"
    );
}

pub(in crate::chat) fn interactive_command_error_json_line(
    input: &str,
    error: &anyhow::Error,
) -> String {
    let command = input.split_whitespace().next().unwrap_or(input);
    let message = error.to_string();
    let error_kind = interactive_chat_turn_error_kind(&message);
    let payload = json!({
        "schema": "ikaros-interactive-command-error-v1",
        "version": 1,
        "command": redact_secrets(command),
        "status": "failed",
        "error_kind": error_kind,
        "message": redact_secrets(&message),
        "recoverable": true,
        "actions": interactive_command_error_actions(command, error_kind),
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-interactive-command-error-v1","version":1,"status":"failed","error_kind":"unknown","message":"failed to serialize error","recoverable":true,"actions":[]}"#
            .to_owned()
    });
    format!("interactive_command_error_json: {encoded}")
}

fn interactive_command_error_actions(command: &str, error_kind: &str) -> serde_json::Value {
    let mut actions = vec![
        json!({
            "label": "help",
            "command": "/help",
            "description": "Show the interactive command list",
        }),
        json!({
            "label": "commands",
            "command": "/commands",
            "description": "Show slash command metadata",
        }),
    ];
    match command {
        "/mcp" => actions.push(json!({
            "label": "mcp_status",
            "command": "/mcp status",
            "description": "Inspect configured MCP servers before probing or calling",
        })),
        "/provider" => actions.push(json!({
            "label": "provider_debug",
            "command": "/provider debug",
            "description": "Inspect provider health, fallback, cooldown, and diagnostics",
        })),
        "/budget" => actions.push(json!({
            "label": "budget_status",
            "command": "/budget",
            "description": "Inspect or update the configured daily token budget",
        })),
        _ => {}
    }
    if error_kind == "provider_error" {
        actions.push(json!({
            "label": "provider_health",
            "command": "/provider health --live",
            "description": "Probe the configured provider through the governed network path",
        }));
    }
    serde_json::Value::Array(actions)
}

pub(in crate::chat) fn print_interactive_chat_turn_error(
    runtime: &InteractiveChatRuntime,
    error: &anyhow::Error,
) {
    if runtime.default_inline_stdout() {
        let _ = terminal::print_inline_history_lines(&[format!(
            "• Turn failed. {}",
            human_error_summary_for_stdout(&redact_secrets(&error.to_string()))
        )]);
        return;
    }
    if runtime.fullscreen_stdout_quiet() {
        return;
    }
    let error_text = error.to_string();
    println!(
        "chat_turn: failed session={} error={}",
        redact_secrets(&runtime.chat_session_id),
        redact_secrets(&error_text)
    );
    println!(
        "{}",
        interactive_chat_turn_error_json_line(&runtime.chat_session_id, error)
    );
    if let Some(hint) = interactive_chat_turn_recovery_hint(error) {
        println!("{hint}");
    }
}

pub(in crate::chat) fn interactive_chat_turn_error_json_line(
    session_id: &str,
    error: &anyhow::Error,
) -> String {
    let message = error.to_string();
    let error_kind = interactive_chat_turn_error_kind(&message);
    let payload = json!({
        "schema": "ikaros-workbench-chat-turn-error-v1",
        "version": 1,
        "session_id": redact_secrets(session_id),
        "status": "failed",
        "error_kind": error_kind,
        "message": redact_secrets(&message),
        "recoverable": error_kind != "unknown",
        "actions": interactive_chat_turn_error_actions(error_kind, &message),
    });
    let encoded = serde_json::to_string(&payload).unwrap_or_else(|_| {
        r#"{"schema":"ikaros-workbench-chat-turn-error-v1","version":1,"status":"failed","error_kind":"unknown","message":"failed to serialize error","recoverable":false,"actions":[]}"#
            .to_owned()
    });
    format!("chat_turn_error_json: {encoded}")
}

pub(in crate::chat) fn interactive_chat_turn_recovery_hint(
    error: &anyhow::Error,
) -> Option<String> {
    let message = error.to_string();
    match interactive_chat_turn_error_kind(&message) {
        "budget_exceeded" => {
            let suggested = suggested_budget_command(&message)
                .unwrap_or_else(|| "/budget set <tokens>".into());
            Some(format!(
                "chat_turn_recovery_hint: /status shows status_model_budget; use {suggested} or /budget disable to update model.default.daily_token_budget"
            ))
        }
        "provider_error" => Some(
            "chat_turn_recovery_hint: /provider debug explains fallback/cooldown; /provider health --live probes the configured provider"
                .into(),
        ),
        "unsupported_content" => Some(
            "chat_turn_recovery_hint: use /attach list to inspect pending attachments, /attach clear to remove them, or switch to a provider/model with matching image/audio/file input support"
                .into(),
        ),
        _ => None,
    }
}

pub(in crate::chat) fn interactive_chat_turn_error_kind(message: &str) -> &'static str {
    let lower = message.to_ascii_lowercase();
    if lower.contains("daily token budget") || lower.contains("token budget exceeded") {
        "budget_exceeded"
    } else if lower.contains("does not support")
        && lower.contains("content block")
        && (lower.contains("image") || lower.contains("audio") || lower.contains("file"))
    {
        "unsupported_content"
    } else if lower.contains("provider")
        || lower.contains("http")
        || lower.contains("rate limit")
        || lower.contains("timeout")
        || lower.contains("dns")
        || lower.contains("network egress")
        || lower.contains("failed to lookup")
        || lower.contains("lookup address")
        || lower.contains("temporary failure in name resolution")
    {
        "provider_error"
    } else {
        "unknown"
    }
}

fn human_error_summary_for_stdout(detail: &str) -> String {
    let lower = detail.to_ascii_lowercase();
    if lower.contains("tools.function.parameters") || lower.contains("tool schema") {
        return "Provider rejected the tool schema. Try again after compatibility is fixed.".into();
    }
    if lower.contains("http 400") || lower.contains("bad request") {
        return "Provider rejected the request (HTTP 400).".into();
    }
    if lower.contains("rate limit") || lower.contains("429") {
        return "Provider rate limit hit. Try again later or switch model.".into();
    }
    if lower.contains("timeout") {
        return "Provider request timed out. Try again or switch model.".into();
    }
    if lower.contains("dns") || lower.contains("network") {
        return "Network/provider connection failed.".into();
    }
    let mut output = String::new();
    for (index, ch) in detail.chars().filter(|ch| !ch.is_control()).enumerate() {
        if index >= 180 {
            output.push_str("...");
            break;
        }
        output.push(ch);
    }
    output
}

pub(in crate::chat) fn interactive_chat_turn_error_actions(
    error_kind: &str,
    message: &str,
) -> serde_json::Value {
    match error_kind {
        "budget_exceeded" => {
            let suggested = suggested_budget_command(message);
            json!([
                {
                    "label": "show_status",
                    "command": "/status",
                    "description": "Inspect status_model_budget and current daily usage",
                },
                {
                    "label": "show_budget",
                    "command": "/budget",
                    "config_key": "model.default.daily_token_budget",
                    "description": "Inspect the configured daily token budget",
                },
                {
                    "label": "raise_budget",
                    "command": "/budget set <tokens>",
                    "config_key": "model.default.daily_token_budget",
                    "description": "Raise the daily token budget in config.yaml",
                },
                {
                    "label": "raise_budget_suggested",
                    "command": suggested.as_deref(),
                    "config_key": "model.default.daily_token_budget",
                    "description": "Raise the daily token budget using a value inferred from the failed request",
                },
                {
                    "label": "disable_budget",
                    "command": "/budget disable",
                    "config_key": "model.default.daily_token_budget",
                    "description": "Disable the daily token budget in config.yaml",
                },
            ])
        }
        "provider_error" => json!([
            {
                "label": "show_provider_debug",
                "command": "/provider debug",
                "description": "Inspect provider health, fallback, cooldown, and diagnostics",
            },
            {
                "label": "show_status",
                "command": "/status",
                "description": "Inspect model runtime and policy state",
            },
        ]),
        "unsupported_content" => json!([
            {
                "label": "show_attachments",
                "command": "/attach list",
                "description": "Inspect pending image, audio, or file content blocks",
            },
            {
                "label": "clear_attachments",
                "command": "/attach clear",
                "description": "Remove pending attachments and retry the text-only turn",
            },
            {
                "label": "show_provider_matrix",
                "command": "/provider matrix",
                "description": "Inspect image_input, audio_input, and file_input support",
            },
        ]),
        _ => json!([
            {
                "label": "show_trace",
                "command": "/trace",
                "description": "Inspect the current session timeline for persisted evidence",
            },
        ]),
    }
}

pub(in crate::chat) fn suggested_budget_command(message: &str) -> Option<String> {
    let used = parse_u64_after_marker(message, "used ")?;
    let estimated = parse_u64_after_marker(message, "estimated request ")?;
    let current = parse_u64_after_marker(message, "budget ")?;
    let required = used.saturating_add(estimated);
    let request_headroom = estimated.max(50_000);
    let suggested = current
        .saturating_mul(2)
        .max(required.saturating_add(request_headroom))
        .max(100_000);
    Some(format!("/budget set {suggested}"))
}

fn parse_u64_after_marker(message: &str, marker: &str) -> Option<u64> {
    let start = message.find(marker)? + marker.len();
    let digits = message[start..]
        .chars()
        .skip_while(|ch| ch.is_ascii_whitespace())
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}
