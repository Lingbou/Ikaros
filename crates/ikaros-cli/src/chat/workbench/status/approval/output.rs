// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::interactive::InteractiveChatRuntime;
use anyhow::Result;
use ikaros_execution::harness::{ApprovalRecord, ApprovalStatus};

use super::super::super::{path_display, terminal_inline};
use super::json::approval_overlay_json_line;
use super::value::{approval_bool, approval_str, approval_u64};

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

pub(in crate::chat::workbench::status) fn print_approval_overlay(
    runtime: &InteractiveChatRuntime,
    pending: &[ApprovalRecord],
) {
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
