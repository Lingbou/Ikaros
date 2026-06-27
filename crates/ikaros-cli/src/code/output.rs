// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use ikaros_core::{ToolResult, redact_secrets};

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
