// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::{RiskLevel, redact_json};
use ikaros_execution::harness::ApprovalRecord;

use super::super::super::terminal_inline;

pub(super) fn approval_scope(record: &ApprovalRecord) -> String {
    record
        .request
        .context
        .as_ref()
        .and_then(|context| context.get("scope"))
        .and_then(serde_json::Value::as_str)
        .map(terminal_inline)
        .or_else(|| {
            record
                .request
                .workspace_root
                .as_ref()
                .map(|_| "workspace".to_owned())
        })
        .unwrap_or_else(|| "session".to_owned())
}

pub(super) fn approval_operations(record: &ApprovalRecord) -> Vec<String> {
    let context = record.request.context.as_ref();
    let mut operations = Vec::new();
    if approval_bool(context, &["operations", "provider_call"]) {
        operations.push("provider".to_owned());
    }
    if approval_bool(context, &["operations", "workspace_write"])
        || matches!(
            record.request.call.risk,
            RiskLevel::LocalWrite
                | RiskLevel::ShellWrite
                | RiskLevel::DatabaseWrite
                | RiskLevel::SelfModify
        )
    {
        operations.push("write".to_owned());
    }
    if approval_bool(context, &["operations", "shell"])
        || matches!(
            record.request.call.risk,
            RiskLevel::ShellRead | RiskLevel::ShellWrite | RiskLevel::Destructive
        )
    {
        operations.push("shell".to_owned());
    }
    if approval_bool(context, &["operations", "network"])
        || matches!(
            record.request.call.risk,
            RiskLevel::Network | RiskLevel::RemoteServer
        )
    {
        operations.push("network".to_owned());
    }
    if approval_bool(context, &["operations", "plugin"])
        || record.request.call.name.starts_with("plugin_")
    {
        operations.push("plugin".to_owned());
    }
    if matches!(record.request.call.risk, RiskLevel::SecretAccess) {
        operations.push("secret".to_owned());
    }
    if matches!(record.request.call.risk, RiskLevel::SelfModify)
        || record.request.call.name.contains("self_modify")
    {
        operations.push("self_modify".to_owned());
    }
    if operations.is_empty() {
        operations.push("read".to_owned());
    }
    operations
}

pub(super) fn approval_provider(record: &ApprovalRecord) -> String {
    let context = record.request.context.as_ref();
    approval_str(context, &["provider", "name"])
        .or_else(|| approval_str(context, &["provider", "provider"]))
        .map(terminal_inline)
        .unwrap_or_else(|| "none".into())
}

pub(super) fn approval_session_id(record: &ApprovalRecord) -> String {
    let context = record.request.context.as_ref();
    approval_str(context, &["session", "session_id"])
        .map(terminal_inline)
        .unwrap_or_else(|| "generated".into())
}

pub(super) fn approval_turn_id(record: &ApprovalRecord) -> String {
    let context = record.request.context.as_ref();
    approval_str(context, &["session", "turn_id"])
        .map(terminal_inline)
        .unwrap_or_else(|| "generated".into())
}

pub(super) fn approval_write_targets(record: &ApprovalRecord) -> String {
    let context = record.request.context.as_ref();
    let mut targets = Vec::new();
    collect_string_array(context, &["operations", "write_targets"], &mut targets);
    collect_string_array(context, &["patch", "paths"], &mut targets);
    collect_string_array(context, &["workspace", "paths"], &mut targets);
    collect_input_path(&record.request.call.input, &mut targets);
    if targets.is_empty() {
        return "none".into();
    }
    targets.sort();
    targets.dedup();
    let joined = targets
        .into_iter()
        .take(4)
        .map(|value| terminal_inline(&value))
        .collect::<Vec<_>>()
        .join("|");
    super::super::truncate_chars(&joined, 160)
}

pub(super) fn approval_shell_command_summary(record: &ApprovalRecord) -> String {
    let context = record.request.context.as_ref();
    let commands = context
        .and_then(|context| context.pointer("/operations/shell_commands"))
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|command| {
            command
                .get("command")
                .and_then(serde_json::Value::as_str)
                .or_else(|| command.as_str())
        })
        .map(terminal_inline)
        .collect::<Vec<_>>();
    if commands.is_empty() {
        return "none".into();
    }
    super::super::truncate_chars(&commands.join("|"), 180)
}

pub(super) fn approval_risk_level(risk: &RiskLevel) -> &'static str {
    if approval_risk_is_high(risk) {
        "high"
    } else {
        match risk {
            RiskLevel::SafeRead => "low",
            RiskLevel::ShellRead | RiskLevel::Network | RiskLevel::SecretAccess => "medium",
            _ => "elevated",
        }
    }
}

pub(super) fn approval_risk_is_high(risk: &RiskLevel) -> bool {
    matches!(
        risk,
        RiskLevel::ShellWrite
            | RiskLevel::DatabaseWrite
            | RiskLevel::RemoteServer
            | RiskLevel::Destructive
            | RiskLevel::SelfModify
    )
}

pub(super) fn approval_input_preview(record: &ApprovalRecord) -> String {
    const MAX_CHARS: usize = 180;
    let input = serde_json::to_string(&redact_json(record.request.call.input.clone()))
        .unwrap_or_else(|_| terminal_inline(&record.request.call.input.to_string()));
    super::super::truncate_chars(&terminal_inline(&input), MAX_CHARS)
}

fn collect_string_array(
    context: Option<&serde_json::Value>,
    path: &[&str],
    output: &mut Vec<String>,
) {
    let Some(values) = approval_value(context, path).and_then(serde_json::Value::as_array) else {
        return;
    };
    output.extend(
        values
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    );
}

fn collect_input_path(input: &serde_json::Value, output: &mut Vec<String>) {
    for key in [
        "path",
        "file",
        "file_path",
        "target",
        "target_path",
        "output_path",
        "destination",
    ] {
        if let Some(value) = input.get(key).and_then(serde_json::Value::as_str) {
            output.push(value.to_owned());
        }
    }
    if let Some(paths) = input.get("paths").and_then(serde_json::Value::as_array) {
        output.extend(
            paths
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned),
        );
    }
}

pub(super) fn approval_bool(context: Option<&serde_json::Value>, path: &[&str]) -> bool {
    approval_value(context, path)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

pub(super) fn approval_str<'a>(
    context: Option<&'a serde_json::Value>,
    path: &[&str],
) -> Option<&'a str> {
    approval_value(context, path).and_then(serde_json::Value::as_str)
}

pub(super) fn approval_u64(context: Option<&serde_json::Value>, path: &[&str]) -> Option<u64> {
    approval_value(context, path).and_then(serde_json::Value::as_u64)
}

pub(super) fn approval_value<'a>(
    context: Option<&'a serde_json::Value>,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut current = context?;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}
