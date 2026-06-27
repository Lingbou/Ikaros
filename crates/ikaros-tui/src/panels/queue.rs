// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn screen_queue_panel_json(screen: &WorkbenchScreen) -> serde_json::Value {
    let bottom = find_cell(screen, |cell| cell.title == "bottom pane");
    let queue = find_cell(screen, |cell| cell.title == "queue");
    let controls = find_cell(screen, |cell| cell.title == "queue controls");
    let continuation_items = screen_queue_continuation_cells(screen)
        .into_iter()
        .map(queue_continuation_item_json)
        .collect::<Vec<_>>();
    let active_item = continuation_items
        .iter()
        .find(|item| {
            item.get("active")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
        })
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let input_items = all_cells(screen)
        .filter(|cell| cell.title.starts_with("input queue "))
        .map(queue_pending_input_item_json)
        .collect::<Vec<_>>();
    let attachment_items = all_cells(screen)
        .filter(|cell| cell.title.starts_with("attachment queue "))
        .map(queue_attachment_item_json)
        .collect::<Vec<_>>();
    let queued = queue
        .and_then(|cell| extract_token_after(&cell.detail, "queued="))
        .unwrap_or_else(|| "0".into());
    let running = queue
        .and_then(|cell| extract_token_after(&cell.detail, "running="))
        .unwrap_or_else(|| "0".into());
    let completed = queue
        .and_then(|cell| extract_token_after(&cell.detail, "completed="))
        .unwrap_or_else(|| "0".into());
    let failed = queue
        .and_then(|cell| extract_token_after(&cell.detail, "failed="))
        .unwrap_or_else(|| "0".into());
    let cancelled = queue
        .and_then(|cell| extract_token_after(&cell.detail, "cancelled="))
        .unwrap_or_else(|| "0".into());
    let pending_inputs = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "pending_inputs="))
        .unwrap_or_else(|| "0".into());
    let needs_attention = failed != "0" || cancelled != "0";
    let primary_command = active_item
        .get("default_action")
        .and_then(serde_json::Value::as_str)
        .filter(|command| *command != "none")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            if pending_inputs != "0" {
                "/queue run".into()
            } else if running != "0" || queued != "0" {
                "/cancel all".into()
            } else {
                "/debug continuations".into()
            }
        });
    let primary_action = action_menu_item_json(
        "queue_primary",
        command_action_label(&primary_command),
        &primary_command,
        command_shortcut(&primary_command),
        command_requires_explicit_action(&primary_command),
    );
    serde_json::json!({
        "bottom_pane": bottom
            .map(screen_bottom_pane_json)
            .unwrap_or_else(|| serde_json::json!({"active_view": "composer"})),
        "queue": queue.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "controls": controls.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "queued": queued.clone(),
        "running": running.clone(),
        "completed": completed.clone(),
        "failed": failed.clone(),
        "cancelled": cancelled.clone(),
        "status_counts": {
            "queued": queued,
            "running": running,
            "completed": completed,
            "failed": failed,
            "cancelled": cancelled,
        },
        "needs_attention": needs_attention,
        "active_item": active_item,
        "primary": primary_action,
        "primary_command": primary_command,
        "continuation_items": continuation_items,
        "input_items": input_items,
        "attachment_items": attachment_items,
        "selection": {
            "model": "list",
            "default_index": 0,
            "selected_item": if active_item.is_null() {
                "none"
            } else {
                "active_continuation"
            },
            "primary_command": primary_command,
            "enter": "open_selected",
            "alt_enter": "confirm_selected",
            "alt_c": "cancel_selected",
            "alt_x": "clear_selected",
        },
        "selection_state": {
            "has_active_item": !active_item.is_null(),
            "has_pending_inputs": pending_inputs != "0",
            "can_run": pending_inputs != "0",
            "can_cancel": running != "0" || queued != "0",
            "can_recover": needs_attention,
            "resume": "/queue run",
            "cancel": "/cancel all",
            "inspect": "/debug continuations",
        },
        "recovery": screen_queue_recovery_json(screen),
        "pending_inputs": pending_inputs,
        "actions": controls
            .or(queue)
            .map(|cell| selected_cell_actions_json(Some(cell), &selected_cell_actions(cell)))
            .unwrap_or_else(|| selected_cell_actions_json(None, &[])),
    })
}

pub(crate) fn screen_queue_continuation_cells(screen: &WorkbenchScreen) -> Vec<&WorkbenchCell> {
    all_cells(screen)
        .filter(|cell| {
            cell.title.starts_with("queue ")
                && !matches!(
                    cell.title.as_str(),
                    "queue" | "queue controls" | "queue recovery"
                )
                && cell.detail.contains("id=")
        })
        .collect()
}

pub(crate) fn queue_continuation_item_json(cell: &WorkbenchCell) -> serde_json::Value {
    let commands = selected_cell_actions(cell);
    let status = extract_token_after(&cell.detail, "status=").unwrap_or_else(|| "unknown".into());
    let active = matches!(status.as_str(), "queued" | "running");
    let retryable = matches!(status.as_str(), "failed" | "cancelled");
    serde_json::json!({
        "kind": cell.kind.as_str(),
        "title": terminal_inline(&cell.title),
        "continuation_kind": cell.title
            .strip_prefix("queue ")
            .map(terminal_inline)
            .unwrap_or_else(|| "unknown".into()),
        "id": extract_token_after(&cell.detail, "id=")
            .unwrap_or_else(|| "unknown".into()),
        "status": status,
        "reason": extract_token_after(&cell.detail, "reason=")
            .unwrap_or_else(|| "none".into()),
        "turn": extract_token_after(&cell.detail, "turn=")
            .unwrap_or_else(|| "none".into()),
        "lease_owner": extract_token_after(&cell.detail, "lease_owner=")
            .unwrap_or_else(|| "none".into()),
        "attempts": extract_token_after(&cell.detail, "attempts=")
            .unwrap_or_else(|| "0".into()),
        "error": extract_token_after(&cell.detail, "error=")
            .unwrap_or_else(|| "none".into()),
        "active": active,
        "terminal": !active,
        "retryable": retryable,
        "default_action": if active {
            command_with_prefix(&commands, "/cancel ")
        } else if retryable {
            command_with_prefix(&commands, "/queue retry ")
                .or_else(|| command_with_prefix(&commands, "/queue requeue "))
        } else {
            command_with_prefix(&commands, "/debug continuations")
        },
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}

pub(crate) fn queue_pending_input_item_json(cell: &WorkbenchCell) -> serde_json::Value {
    let commands = selected_cell_actions(cell);
    serde_json::json!({
        "kind": "pending_input",
        "title": terminal_inline(&cell.title),
        "index": extract_token_after(&cell.detail, "index=")
            .unwrap_or_else(|| "unknown".into()),
        "message": extract_assignment_span(
            &cell.detail,
            "message=",
            &[" command=", " continue_hint=", " clear=", " clear_all="],
        )
        .unwrap_or_else(|| "none".into()),
        "pending_inputs": extract_token_after(&cell.detail, "pending_inputs=")
            .unwrap_or_else(|| "0".into()),
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}

pub(crate) fn queue_attachment_item_json(cell: &WorkbenchCell) -> serde_json::Value {
    let commands = selected_cell_actions(cell);
    serde_json::json!({
        "kind": "attachment",
        "title": terminal_inline(&cell.title),
        "index": extract_token_after(&cell.detail, "index=")
            .unwrap_or_else(|| "unknown".into()),
        "content_kind": extract_token_after(&cell.detail, "kind=")
            .unwrap_or_else(|| "unknown".into()),
        "summary": extract_assignment_span(
            &cell.detail,
            "summary=",
            &[" clear=", " clear_all=", " command="],
        )
        .unwrap_or_else(|| "none".into()),
        "pending_attachments": extract_token_after(&cell.detail, "pending_attachments=")
            .unwrap_or_else(|| "0".into()),
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}

pub(crate) fn screen_queue_recovery_json(screen: &WorkbenchScreen) -> serde_json::Value {
    let recovery = find_cell(screen, |cell| cell.title == "queue recovery");
    let interrupt = find_cell(screen, |cell| cell.title == "interrupt running work");
    serde_json::json!({
        "failed": recovery.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "interrupt": interrupt.map(panel_cell_json).unwrap_or(serde_json::Value::Null),
        "actions": {
            "run": "/queue run",
            "cancel_all": "/cancel all",
            "inspect": "/debug continuations",
            "failed_timeline": "/timeline --failed",
            "failed_trace": "/trace --failed",
        },
    })
}
