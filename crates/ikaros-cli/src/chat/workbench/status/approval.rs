// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_core::{RiskLevel, redact_json};
use ikaros_harness::{ApprovalRecord, ApprovalStatus};
use std::path::Path;

use super::super::{WorkbenchCell, WorkbenchCellKind, path_display, terminal_inline};

pub(in crate::chat) fn print_approval_status(runtime: &InteractiveChatRuntime) -> Result<()> {
    let pending = runtime.session.pending_approvals()?;
    let records = runtime.session.approval_records()?;
    let approved = records
        .iter()
        .filter(|record| record.status == ApprovalStatus::Approved)
        .count();
    let rejected = records
        .iter()
        .filter(|record| record.status == ApprovalStatus::Denied)
        .count();
    println!("approvals_pending: {}", pending.len());
    println!("approvals_total: {}", records.len());
    println!("approvals_approved: {approved}");
    println!("approvals_rejected: {rejected}");
    if let Some(log) = runtime.session.approvals.log() {
        println!("approvals_log: {}", path_display(log.path()));
    } else {
        println!("approvals_log: none");
    }
    print_approval_overlay(runtime, &pending);
    Ok(())
}

pub(super) fn screen_approval_cells(pending: &[ApprovalRecord]) -> Vec<WorkbenchCell> {
    if pending.is_empty() {
        return vec![WorkbenchCell {
            kind: WorkbenchCellKind::Approval,
            title: "approvals".into(),
            detail: "none pending".into(),
        }];
    }
    let mut cells = vec![screen_approval_summary_cell(pending)];
    cells.extend(pending
        .iter()
        .take(4)
        .map(|record| WorkbenchCell {
            kind: WorkbenchCellKind::Approval,
            title: format!("pending {}", terminal_inline(&record.request.id)),
            detail: format!(
                "approval_id={} call_id={} tool={} risk={:?} risk_level={} scope={} operations={} provider={} session={} turn={} write_targets={} shell_commands={} reason={} input_preview={} approve=/approval approve {} deny=/approval deny {} open=/screen open-selected",
                terminal_inline(&record.request.id),
                terminal_inline(&record.request.call.id),
                terminal_inline(&record.request.call.name),
                record.request.call.risk,
                approval_risk_level(&record.request.call.risk),
                approval_scope(record),
                approval_operations(record).join(","),
                approval_provider(record),
                approval_session_id(record),
                approval_turn_id(record),
                approval_write_targets(record),
                approval_shell_command_summary(record),
                terminal_inline(&record.request.reason),
                approval_input_preview(record),
                terminal_inline(&record.request.id),
                terminal_inline(&record.request.id),
            ),
        }));
    cells
}

pub(super) fn print_approval_overlay(runtime: &InteractiveChatRuntime, pending: &[ApprovalRecord]) {
    if pending.is_empty() {
        println!("approval_overlay: none");
        println!(
            "{}",
            approval_overlay_json_line(pending, Some(runtime.workspace.as_path()))
        );
        return;
    }
    println!("approval_overlay:");
    for record in pending {
        let context = record.request.context.as_ref();
        println!(
            "approval_item: id={} tool={} risk={:?} status={:?}",
            terminal_inline(&record.request.id),
            terminal_inline(&record.request.call.name),
            record.request.call.risk,
            record.status
        );
        println!("  reason: {}", terminal_inline(&record.request.reason));
        if let Some(workspace) = record.request.workspace_root.as_ref() {
            println!("  workspace: {}", path_display(workspace));
        } else {
            println!("  workspace: {}", path_display(&runtime.workspace));
        }
        println!(
            "  provider_call: {}",
            approval_bool(context, &["operations", "provider_call"])
        );
        println!(
            "  workspace_write: {}",
            approval_bool(context, &["operations", "workspace_write"])
        );
        let shell_requested = approval_bool(context, &["operations", "shell"]);
        println!("  shell: {shell_requested}");
        print_approval_shell_commands(context, shell_requested);
        println!("  network: {}", runtime.agent.profile.network);
        println!(
            "  provider: {}",
            approval_str(context, &["provider", "name"])
                .map(terminal_inline)
                .unwrap_or_else(|| "not_configured".into())
        );
        println!(
            "  session: {} turn={}",
            approval_str(context, &["session", "session_id"])
                .map(terminal_inline)
                .unwrap_or_else(|| "<generated>".into()),
            approval_str(context, &["session", "turn_id"])
                .map(terminal_inline)
                .unwrap_or_else(|| "<generated>".into())
        );
        println!(
            "  diff_size: {}",
            approval_u64(context, &["patch", "candidate_diff_chars"]).unwrap_or(0)
        );
        println!(
            "  approve: /approval approve {}",
            terminal_inline(&record.request.id)
        );
        println!(
            "  deny: /approval deny {}",
            terminal_inline(&record.request.id)
        );
        println!(
            "  external_replay: ikaros approval approve {}",
            terminal_inline(&record.request.id)
        );
    }
    println!(
        "{}",
        approval_overlay_json_line(pending, Some(runtime.workspace.as_path()))
    );
}

fn screen_approval_summary_cell(pending: &[ApprovalRecord]) -> WorkbenchCell {
    let summary = approval_queue_summary(pending);
    let first = pending
        .first()
        .map(|record| terminal_inline(&record.request.id))
        .unwrap_or_else(|| "none".into());
    WorkbenchCell {
        kind: WorkbenchCellKind::Approval,
        title: "approval controls".into(),
        detail: format!(
            "pending={} high_risk={} provider={} write={} shell={} network={} plugin={} self_modify={} first={} approve=/screen approve-selected deny=/screen deny-selected continue=/queue run list=/approval trace=/trace --approval timeline=/timeline --approval inspect=/screen --focus side --select 2 open=/screen open-selected",
            pending.len(),
            summary.high_risk,
            summary.provider_calls,
            summary.workspace_writes,
            summary.shell_calls,
            summary.network_calls,
            summary.plugin_calls,
            summary.self_modify_calls,
            first,
        ),
    }
}

#[derive(Debug, Default)]
struct ApprovalQueueSummary {
    high_risk: usize,
    provider_calls: usize,
    workspace_writes: usize,
    shell_calls: usize,
    network_calls: usize,
    plugin_calls: usize,
    self_modify_calls: usize,
}

fn approval_queue_summary(pending: &[ApprovalRecord]) -> ApprovalQueueSummary {
    let mut summary = ApprovalQueueSummary::default();
    for record in pending {
        let context = record.request.context.as_ref();
        if approval_risk_is_high(&record.request.call.risk) {
            summary.high_risk += 1;
        }
        if approval_bool(context, &["operations", "provider_call"]) {
            summary.provider_calls += 1;
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
            summary.workspace_writes += 1;
        }
        if approval_bool(context, &["operations", "shell"])
            || matches!(
                record.request.call.risk,
                RiskLevel::ShellRead | RiskLevel::ShellWrite | RiskLevel::Destructive
            )
        {
            summary.shell_calls += 1;
        }
        if approval_bool(context, &["operations", "network"])
            || matches!(
                record.request.call.risk,
                RiskLevel::Network | RiskLevel::RemoteServer
            )
        {
            summary.network_calls += 1;
        }
        if approval_bool(context, &["operations", "plugin"])
            || record.request.call.name.starts_with("plugin_")
        {
            summary.plugin_calls += 1;
        }
        if matches!(record.request.call.risk, RiskLevel::SelfModify)
            || record.request.call.name.contains("self_modify")
        {
            summary.self_modify_calls += 1;
        }
    }
    summary
}

pub(super) fn approval_overlay_json_line(
    pending: &[ApprovalRecord],
    workspace: Option<&Path>,
) -> String {
    let items = pending
        .iter()
        .map(|record| approval_overlay_item_json(record, workspace))
        .collect::<Vec<_>>();
    let summary = approval_queue_summary(pending);
    let primary_id = pending
        .first()
        .map(|record| terminal_inline(&record.request.id))
        .unwrap_or_else(|| "none".into());
    format!(
        "approval_overlay_json: {}",
        serde_json::json!({
            "schema": "ikaros-workbench-approval-overlay-v1",
            "version": 1,
            "pending_count": pending.len(),
            "summary": {
                "high_risk": summary.high_risk,
                "provider_calls": summary.provider_calls,
                "workspace_writes": summary.workspace_writes,
                "shell_calls": summary.shell_calls,
                "network_calls": summary.network_calls,
                "plugin_calls": summary.plugin_calls,
                "self_modify_calls": summary.self_modify_calls,
            },
            "primary_id": primary_id,
            "primary_actions": {
                "approve_selected": (pending.len() > 0).then_some("/screen approve-selected"),
                "deny_selected": (pending.len() > 0).then_some("/screen deny-selected"),
                "continue_after_decision": "/queue run",
                "inspect_selected": "/screen --focus side --select 2 open-selected",
                "trace": "/trace --approval",
                "timeline": "/timeline --approval",
            },
            "items": items,
        })
    )
}

fn approval_overlay_item_json(
    record: &ApprovalRecord,
    workspace: Option<&Path>,
) -> serde_json::Value {
    let context = record.request.context.clone().map(redact_json);
    let workspace = record
        .request
        .workspace_root
        .as_deref()
        .or(workspace)
        .map(path_display)
        .unwrap_or_else(|| "none".into());
    serde_json::json!({
        "id": terminal_inline(&record.request.id),
        "tool": terminal_inline(&record.request.call.name),
        "risk": format!("{:?}", record.request.call.risk),
        "risk_level": approval_risk_level(&record.request.call.risk),
        "status": format!("{:?}", record.status),
        "reason": terminal_inline(&record.request.reason),
        "workspace": workspace,
        "created_at": terminal_inline(&record.request.created_at),
        "updated_at": terminal_inline(&record.updated_at),
        "scope": approval_scope(record),
        "operations": approval_operations(record),
        "provider": approval_provider(record),
        "session_id": approval_session_id(record),
        "turn_id": approval_turn_id(record),
        "write_targets": approval_write_targets(record),
        "shell_commands": approval_shell_command_summary(record),
        "call": {
            "id": terminal_inline(&record.request.call.id),
            "input": redact_json(record.request.call.input.clone()),
        },
        "context": context.unwrap_or(serde_json::Value::Null),
        "actions": {
            "approve": format!("/approval approve {}", terminal_inline(&record.request.id)),
            "deny": format!("/approval deny {}", terminal_inline(&record.request.id)),
            "approve_selected": "/screen approve-selected",
            "deny_selected": "/screen deny-selected",
            "continue_after_decision": "/queue run",
            "inspect": format!("/screen --focus side --select-title pending {}", terminal_inline(&record.request.id)),
            "trace": "/trace --approval",
            "timeline": "/timeline --approval",
            "external_replay": format!("ikaros approval approve {}", terminal_inline(&record.request.id)),
        },
    })
}

fn print_approval_shell_commands(context: Option<&serde_json::Value>, shell_requested: bool) {
    let commands = context
        .and_then(|context| context.pointer("/operations/shell_commands"))
        .and_then(serde_json::Value::as_array);
    match commands {
        Some(commands) if !commands.is_empty() => {
            println!("  shell_commands:");
            for command in commands {
                let command_text = command["command"].as_str().unwrap_or("<unknown>");
                let reason = command["reason"].as_str().unwrap_or("unspecified");
                println!(
                    "    - {} ({})",
                    terminal_inline(command_text),
                    terminal_inline(reason)
                );
            }
        }
        _ => {
            let inferred = approval_bool(context, &["operations", "shell_commands_inferred"]);
            if shell_requested && inferred {
                println!("  shell_commands: inferred from workspace");
            } else {
                println!("  shell_commands: none");
            }
        }
    }
}

fn approval_scope(record: &ApprovalRecord) -> String {
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

fn approval_operations(record: &ApprovalRecord) -> Vec<String> {
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

fn approval_provider(record: &ApprovalRecord) -> String {
    let context = record.request.context.as_ref();
    approval_str(context, &["provider", "name"])
        .or_else(|| approval_str(context, &["provider", "provider"]))
        .map(terminal_inline)
        .unwrap_or_else(|| "none".into())
}

fn approval_session_id(record: &ApprovalRecord) -> String {
    let context = record.request.context.as_ref();
    approval_str(context, &["session", "session_id"])
        .map(terminal_inline)
        .unwrap_or_else(|| "generated".into())
}

fn approval_turn_id(record: &ApprovalRecord) -> String {
    let context = record.request.context.as_ref();
    approval_str(context, &["session", "turn_id"])
        .map(terminal_inline)
        .unwrap_or_else(|| "generated".into())
}

fn approval_write_targets(record: &ApprovalRecord) -> String {
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
    super::truncate_chars(&joined, 160)
}

fn approval_shell_command_summary(record: &ApprovalRecord) -> String {
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
    super::truncate_chars(&commands.join("|"), 180)
}

fn approval_risk_level(risk: &RiskLevel) -> &'static str {
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

fn approval_risk_is_high(risk: &RiskLevel) -> bool {
    matches!(
        risk,
        RiskLevel::ShellWrite
            | RiskLevel::DatabaseWrite
            | RiskLevel::RemoteServer
            | RiskLevel::Destructive
            | RiskLevel::SelfModify
    )
}

fn approval_input_preview(record: &ApprovalRecord) -> String {
    const MAX_CHARS: usize = 180;
    let input = serde_json::to_string(&redact_json(record.request.call.input.clone()))
        .unwrap_or_else(|_| terminal_inline(&record.request.call.input.to_string()));
    super::truncate_chars(&terminal_inline(&input), MAX_CHARS)
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

fn approval_bool(context: Option<&serde_json::Value>, path: &[&str]) -> bool {
    approval_value(context, path)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

fn approval_str<'a>(context: Option<&'a serde_json::Value>, path: &[&str]) -> Option<&'a str> {
    approval_value(context, path).and_then(serde_json::Value::as_str)
}

fn approval_u64(context: Option<&serde_json::Value>, path: &[&str]) -> Option<u64> {
    approval_value(context, path).and_then(serde_json::Value::as_u64)
}

fn approval_value<'a>(
    context: Option<&'a serde_json::Value>,
    path: &[&str],
) -> Option<&'a serde_json::Value> {
    let mut current = context?;
    for segment in path {
        current = current.get(*segment)?;
    }
    Some(current)
}
