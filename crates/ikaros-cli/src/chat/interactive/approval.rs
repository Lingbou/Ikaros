// SPDX-License-Identifier: GPL-3.0-only

use crate::approval::{ApprovalCommand, approval_command};
use anyhow::Result;
use ikaros_core::IkarosPaths;
use serde_json::json;
use std::path::Path;

use super::evidence::append_workbench_evidence_with_text;
use super::{InteractiveChatRuntime, terminal_inline};
use crate::chat::workbench::print_approval_status;

pub(super) async fn handle_approval_command(
    args: Vec<&str>,
    paths: &IkarosPaths,
    workspace: &Path,
    runtime: &InteractiveChatRuntime,
    source: &str,
) -> Result<()> {
    match args.as_slice() {
        [] | ["list"] => {
            print_approval_status(runtime)?;
        }
        ["approve", id, rest @ ..] => {
            let note = approval_note(rest);
            let replay_status = approval_replay_status(runtime, id)?;
            println!("workbench_approval_decision: approved");
            println!("workbench_approval_id: {}", terminal_inline(id));
            approval_command(
                ApprovalCommand::Approve {
                    id: (*id).to_owned(),
                    note,
                },
                paths,
                workspace,
                Some(&runtime.agent.name),
            )
            .await?;
            println!("workbench_approval_replay: {replay_status}");
            append_workbench_approval_evidence(runtime, source, "approved", id, replay_status)?;
            print_approval_continuation_hint(runtime, replay_status)?;
            print_approval_status(runtime)?;
        }
        ["deny", id, rest @ ..] => {
            let note = approval_note(rest);
            println!("workbench_approval_decision: denied");
            println!("workbench_approval_id: {}", terminal_inline(id));
            approval_command(
                ApprovalCommand::Deny {
                    id: (*id).to_owned(),
                    note,
                },
                paths,
                workspace,
                Some(&runtime.agent.name),
            )
            .await?;
            println!("workbench_approval_replay: denied");
            append_workbench_approval_evidence(runtime, source, "denied", id, "denied")?;
            print_approval_continuation_hint(runtime, "denied")?;
            print_approval_status(runtime)?;
        }
        _ => {
            println!("usage: /approval [approve|deny <id> [note]]");
            print_approval_status(runtime)?;
        }
    }
    Ok(())
}

fn append_workbench_approval_evidence(
    runtime: &InteractiveChatRuntime,
    source: &str,
    decision: &str,
    approval_id: &str,
    replay_status: &str,
) -> Result<()> {
    let pending = runtime.session.pending_approvals()?;
    let resume = pending
        .first()
        .map(|record| format!("/approval approve {}", terminal_inline(&record.request.id)));
    append_workbench_evidence_with_text(
        runtime,
        "approval",
        format!("workbench approval {decision}"),
        json!({
            "source": source,
            "decision": decision,
            "approval_id": terminal_inline(approval_id),
            "replay_status": terminal_inline(replay_status),
            "pending_after": pending.len(),
            "next": {
                "screen": "/screen",
                "timeline": "/timeline",
                "trace": "/trace",
                "resume": resume.unwrap_or_else(|| "none".into()),
            },
        }),
    )
}

fn print_approval_continuation_hint(
    runtime: &InteractiveChatRuntime,
    replay_status: &str,
) -> Result<()> {
    let pending = runtime.session.pending_approvals()?;
    let next_resume = pending
        .first()
        .map(|record| format!("/approval approve {}", terminal_inline(&record.request.id)));
    println!("workbench_approval_next: screen=/screen timeline=/timeline trace=/trace");
    println!(
        "workbench_approval_continue: status={} next=/screen timeline=/timeline trace=/trace pending={}",
        terminal_inline(replay_status),
        pending.len()
    );
    if let Some(next) = pending.first() {
        println!(
            "workbench_approval_resume: /approval approve {}",
            terminal_inline(&next.request.id)
        );
    } else {
        println!("workbench_approval_resume: none");
    }
    println!(
        "{}",
        approval_continue_json_line(replay_status, pending.len(), next_resume.as_deref())
    );
    Ok(())
}

fn approval_continue_json_line(
    replay_status: &str,
    pending_count: usize,
    resume: Option<&str>,
) -> String {
    let auto_continue_status = match (replay_status, pending_count) {
        ("executed", 0) => "completed",
        ("denied", _) => "stopped",
        ("approved_not_executed", _) => "manual_apply_required",
        (_, count) if count > 0 => "pending_more_approvals",
        _ => "unknown",
    };
    format!(
        "workbench_approval_continue_json: {}",
        serde_json::json!({
            "schema": "ikaros-workbench-approval-continue-v1",
            "version": 1,
            "replay_status": terminal_inline(replay_status),
            "pending_count": pending_count,
            "auto_continue_status": auto_continue_status,
            "actions": {
                "screen": "/screen",
                "timeline": "/timeline",
                "trace": "/trace",
                "resume": resume.map(terminal_inline),
            }
        })
    )
}

fn approval_note(parts: &[&str]) -> Option<String> {
    let note = parts.join(" ");
    if note.trim().is_empty() {
        None
    } else {
        Some(note)
    }
}

fn approval_replay_status(runtime: &InteractiveChatRuntime, id: &str) -> Result<&'static str> {
    let status = runtime
        .session
        .approvals
        .get(id)?
        .map(|record| {
            if record.request.call.name == "self_modify_apply" {
                "approved_not_executed"
            } else {
                "executed"
            }
        })
        .unwrap_or("unknown");
    Ok(status)
}
