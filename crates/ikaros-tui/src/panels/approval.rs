// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn screen_approval_panel_json(screen: &WorkbenchScreen) -> serde_json::Value {
    let controls = find_cell(screen, |cell| cell.title == "approval controls");
    let placeholder = find_cell(screen, |cell| cell.title == "approvals");
    let pending_items = all_cells(screen)
        .filter(|cell| {
            matches!(cell.kind, WorkbenchCellKind::Approval) && cell.title.starts_with("pending ")
        })
        .map(approval_pending_item_json)
        .collect::<Vec<_>>();
    let primary = pending_items
        .first()
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let pending = controls
        .and_then(|cell| extract_token_after(&cell.detail, "pending="))
        .unwrap_or_else(|| pending_items.len().to_string());
    let needs_attention = pending != "0";
    serde_json::json!({
        "summary": controls
            .map(panel_cell_json)
            .or_else(|| placeholder.map(panel_cell_json))
            .unwrap_or(serde_json::Value::Null),
        "pending": pending,
        "needs_attention": needs_attention,
        "high_risk": controls
            .and_then(|cell| extract_token_after(&cell.detail, "high_risk="))
            .unwrap_or_else(|| "0".into()),
        "provider_calls": controls
            .and_then(|cell| extract_token_after(&cell.detail, "provider="))
            .unwrap_or_else(|| "0".into()),
        "workspace_writes": controls
            .and_then(|cell| extract_token_after(&cell.detail, "write="))
            .unwrap_or_else(|| "0".into()),
        "shell_calls": controls
            .and_then(|cell| extract_token_after(&cell.detail, "shell="))
            .unwrap_or_else(|| "0".into()),
        "network_calls": controls
            .and_then(|cell| extract_token_after(&cell.detail, "network="))
            .unwrap_or_else(|| "0".into()),
        "plugin_calls": controls
            .and_then(|cell| extract_token_after(&cell.detail, "plugin="))
            .unwrap_or_else(|| "0".into()),
        "self_modify_calls": controls
            .and_then(|cell| extract_token_after(&cell.detail, "self_modify="))
            .unwrap_or_else(|| "0".into()),
        "primary": primary,
        "pending_items": pending_items,
        "decision_model": approval_decision_model_json(
            &pending,
            controls,
            placeholder,
            primary.clone(),
        ),
        "approve_selected": "/screen approve-selected",
        "deny_selected": "/screen deny-selected",
        "list_action": "/approval",
        "trace_action": "/trace --approval",
        "timeline_action": "/timeline --approval",
        "inspect_action": controls
            .and_then(|cell| command_with_prefix(&selected_cell_actions(cell), "/screen --focus side"))
            .unwrap_or_else(|| "/screen --focus side --select-title pending".into()),
        "actions": controls
            .or(placeholder)
            .map(|cell| selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)))
            .unwrap_or_else(|| selected_cell_actions_json(None, &[])),
    })
}

pub(crate) fn screen_approval_decision_model_json(screen: &WorkbenchScreen) -> serde_json::Value {
    screen_approval_panel_json(screen)
        .get("decision_model")
        .cloned()
        .unwrap_or_else(|| {
            serde_json::json!({
                "schema": "ikaros-inline-approval-decision-v1",
                "status": "idle",
                "pending": "0",
                "primary_id": "none",
                "decisions": [],
            })
        })
}

pub(crate) fn approval_decision_model_json(
    pending: &str,
    controls: Option<&WorkbenchCell>,
    placeholder: Option<&WorkbenchCell>,
    primary: serde_json::Value,
) -> serde_json::Value {
    let summary_source = controls.or(placeholder);
    let high_risk = summary_source
        .and_then(|cell| extract_token_after(&cell.detail, "high_risk="))
        .unwrap_or_else(|| "0".into());
    let provider = summary_source
        .and_then(|cell| extract_token_after(&cell.detail, "provider="))
        .unwrap_or_else(|| "0".into());
    let write = summary_source
        .and_then(|cell| extract_token_after(&cell.detail, "write="))
        .unwrap_or_else(|| "0".into());
    let shell = summary_source
        .and_then(|cell| extract_token_after(&cell.detail, "shell="))
        .unwrap_or_else(|| "0".into());
    let network = summary_source
        .and_then(|cell| extract_token_after(&cell.detail, "network="))
        .unwrap_or_else(|| "0".into());
    let plugin = summary_source
        .and_then(|cell| extract_token_after(&cell.detail, "plugin="))
        .unwrap_or_else(|| "0".into());
    let self_modify = summary_source
        .and_then(|cell| extract_token_after(&cell.detail, "self_modify="))
        .unwrap_or_else(|| "0".into());
    let has_provider = provider != "0";
    let has_write = write != "0";
    let has_shell = shell != "0";
    let has_network = network != "0";
    let has_plugin = plugin != "0";
    let has_self_modify = self_modify != "0";
    let primary_id = primary
        .get("approval_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none")
        .to_owned();
    serde_json::json!({
        "schema": "ikaros-inline-approval-decision-v1",
        "status": if pending == "0" { "idle" } else { "pending" },
        "pending": pending,
        "primary_id": primary_id,
        "primary": primary,
        "risk_breakdown": {
            "high_risk": high_risk,
            "provider_calls": provider,
            "workspace_writes": write,
            "shell_calls": shell,
            "network_calls": network,
            "plugin_calls": plugin,
            "self_modify_calls": self_modify,
        },
        "operations": {
            "provider": has_provider,
            "workspace_write": has_write,
            "shell": has_shell,
            "network": has_network,
            "plugin": has_plugin,
            "self_modify": has_self_modify,
        },
        "decisions": [
            {
                "id": "inspect",
                "label": "Inspect",
                "key": "enter",
                "command": "/approval",
                "risk": "read",
                "requires_explicit_action": false,
            },
            {
                "id": "approve",
                "label": "Approve",
                "key": "alt+a",
                "command": "/screen approve-selected",
                "risk": "approval-decision",
                "requires_explicit_action": true,
                "continues_after_decision": true,
                "continue_command": "/queue run",
            },
            {
                "id": "deny",
                "label": "Deny",
                "key": "alt+d",
                "command": "/screen deny-selected",
                "risk": "approval-decision",
                "requires_explicit_action": true,
                "continues_after_decision": false,
            },
        ],
        "guardrails": {
            "decision_must_name_approval_id": true,
            "approval_replay_bound_to_session": true,
            "redacted_input_preview": true,
            "audit_required": true,
        },
        "actions": {
            "approve_selected": "/screen approve-selected",
            "deny_selected": "/screen deny-selected",
            "list": "/approval",
            "trace": "/trace --approval",
            "timeline": "/timeline --approval",
            "continue": "/queue run",
        },
    })
}

pub(crate) fn approval_pending_item_json(cell: &WorkbenchCell) -> serde_json::Value {
    let commands = selected_cell_actions(cell);
    serde_json::json!({
        "kind": cell.kind.as_str(),
        "title": terminal_inline(&cell.title),
        "approval_id": extract_token_after(&cell.detail, "approval_id=")
            .or_else(|| cell.title.strip_prefix("pending ").map(terminal_inline))
            .unwrap_or_else(|| "unknown".into()),
        "call_id": extract_token_after(&cell.detail, "call_id=")
            .unwrap_or_else(|| "unknown".into()),
        "tool": extract_token_after(&cell.detail, "tool=")
            .unwrap_or_else(|| "unknown".into()),
        "risk_level": extract_token_after(&cell.detail, "risk_level=")
            .unwrap_or_else(|| "unknown".into()),
        "scope": extract_token_after(&cell.detail, "scope=")
            .unwrap_or_else(|| "unknown".into()),
        "operations": extract_token_after(&cell.detail, "operations=")
            .unwrap_or_else(|| "none".into()),
        "provider": extract_token_after(&cell.detail, "provider=")
            .unwrap_or_else(|| "unknown".into()),
        "session": extract_token_after(&cell.detail, "session=")
            .unwrap_or_else(|| "unknown".into()),
        "turn": extract_token_after(&cell.detail, "turn=")
            .unwrap_or_else(|| "unknown".into()),
        "write_targets": extract_token_after(&cell.detail, "write_targets=")
            .unwrap_or_else(|| "0".into()),
        "shell_commands": extract_token_after(&cell.detail, "shell_commands=")
            .unwrap_or_else(|| "0".into()),
        "reason": extract_assignment_span(&cell.detail, "reason=", &[" input_preview=", " approve="])
            .unwrap_or_else(|| "none".into()),
        "input_preview": extract_assignment_span(&cell.detail, "input_preview=", &[" approve=", " deny="])
            .unwrap_or_else(|| "none".into()),
        "approve": command_with_prefix(&commands, "/approval approve "),
        "deny": command_with_prefix(&commands, "/approval deny "),
        "open": command_with_prefix(&commands, "/screen open-selected"),
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}
