// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::redact_json;
use ikaros_execution::harness::ApprovalRecord;
use std::path::Path;

use super::super::super::{path_display, terminal_inline};
use super::cells::approval_queue_summary;
use super::value::{
    approval_operations, approval_provider, approval_risk_level, approval_scope,
    approval_session_id, approval_shell_command_summary, approval_turn_id, approval_write_targets,
};

pub(in crate::chat::workbench::status) fn approval_overlay_json_line(
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
