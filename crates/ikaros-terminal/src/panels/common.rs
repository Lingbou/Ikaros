// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn panel_cell_json(cell: &WorkbenchCell) -> serde_json::Value {
    let actions = selected_cell_actions(cell);
    serde_json::json!({
        "kind": cell.kind.as_str(),
        "title": terminal_inline(&cell.title),
        "detail": terminal_inline(&cell.detail),
        "actions": selected_cell_actions_json(Some(cell), &actions),
    })
}

pub(crate) fn find_cell<'a>(
    screen: &'a WorkbenchScreen,
    predicate: impl Fn(&WorkbenchCell) -> bool,
) -> Option<&'a WorkbenchCell> {
    all_cells(screen).find(|cell| predicate(*cell))
}

pub(crate) fn all_cells<'a>(
    screen: &'a WorkbenchScreen,
) -> impl Iterator<Item = &'a WorkbenchCell> {
    screen
        .status
        .iter()
        .chain(screen.timeline.iter())
        .chain(screen.main.iter())
        .chain(screen.side.iter())
}

pub(crate) fn screen_bottom_pane_json(cell: &WorkbenchCell) -> serde_json::Value {
    let active_view =
        extract_token_after(&cell.detail, "active_view=").unwrap_or_else(|| "composer".into());
    let modal_stack = bottom_pane_modal_stack(&active_view);
    let view_stack = bottom_pane_view_stack_json(&active_view, cell);
    let requires_action = bottom_pane_view_requires_action(&active_view);
    serde_json::json!({
        "kind": cell.kind.as_str(),
        "title": terminal_inline(&cell.title),
        "active_view": active_view.clone(),
        "modal_stack": modal_stack,
        "view_stack": view_stack,
        "active_view_requires_action": requires_action,
        "terminal_title": if requires_action {
            "Action Required"
        } else {
            "Ikaros"
        },
        "input_routing": {
            "composer_retained": true,
            "active_view_first": true,
            "enter": if requires_action { "accept_active_view" } else { "submit_or_open_selected" },
            "esc": bottom_pane_escape_behavior(&active_view),
            "ctrl_c": bottom_pane_cancel_behavior(&active_view),
        },
        "approvals": extract_token_after(&cell.detail, "approvals=")
            .unwrap_or_else(|| "0".into()),
        "pending_inputs": extract_token_after(&cell.detail, "pending_inputs=")
            .unwrap_or_else(|| "0".into()),
        "next_input": extract_assignment_span(&cell.detail, "next_input=", &[" attachments="])
            .unwrap_or_else(|| "none".into()),
        "attachments": extract_token_after(&cell.detail, "attachments=")
            .unwrap_or_else(|| "0".into()),
        "continuations": extract_token_after(&cell.detail, "continuations=")
            .unwrap_or_else(|| "0".into()),
        "composer": {
            "input": extract_token_after(&cell.detail, "input=")
                .unwrap_or_else(|| "readline".into()),
            "completion": extract_token_after(&cell.detail, "tab=")
                .unwrap_or_else(|| "complete".into()),
            "history": extract_token_after(&cell.detail, "ctrl-r=")
                .unwrap_or_else(|| "history".into()),
            "palette": extract_assignment_display(&cell.detail, "palette=", "/screen --palette"),
        },
    })
}

pub(crate) fn bottom_pane_modal_stack(active_view: &str) -> Vec<&'static str> {
    match active_view {
        "approval" => vec!["approval"],
        "input_queue" => vec!["input_queue"],
        "attachments" => vec!["attachments"],
        "continuation" => vec!["continuation"],
        _ => Vec::new(),
    }
}

pub(crate) fn bottom_pane_view_stack_json(
    active_view: &str,
    cell: &WorkbenchCell,
) -> Vec<serde_json::Value> {
    let mut views = vec![serde_json::json!({
        "id": "composer",
        "kind": "composer",
        "focus": active_view == "composer",
        "retained": true,
        "input_enabled": active_view == "composer",
        "requires_action": false,
        "blocks_composer": false,
        "actions": {
            "submit": "enter",
            "complete": "tab",
            "history": "ctrl-r",
            "palette": extract_assignment_display(&cell.detail, "palette=", "/screen --palette"),
        },
    })];
    if active_view != "composer" {
        views.push(serde_json::json!({
            "id": active_view,
            "kind": active_view,
            "focus": true,
            "retained": false,
            "input_enabled": false,
            "requires_action": bottom_pane_view_requires_action(active_view),
            "blocks_composer": true,
            "terminal_title_requires_action": bottom_pane_view_requires_action(active_view),
            "cancel_behavior": bottom_pane_cancel_behavior(active_view),
            "escape_behavior": bottom_pane_escape_behavior(active_view),
            "completion_actions": bottom_pane_completion_actions_json(active_view, cell),
        }));
    }
    views
}

pub(crate) fn bottom_pane_view_requires_action(active_view: &str) -> bool {
    matches!(active_view, "approval" | "input_queue" | "attachments")
}

pub(crate) fn bottom_pane_cancel_behavior(active_view: &str) -> &'static str {
    match active_view {
        "approval" => "deny_or_dismiss_overlay",
        "input_queue" => "cancel_pending_input",
        "attachments" => "clear_or_dismiss_attachments",
        "continuation" => "cancel_continuation",
        _ => "interrupt_or_quit",
    }
}

pub(crate) fn bottom_pane_escape_behavior(active_view: &str) -> &'static str {
    match active_view {
        "approval" => "route_to_approval_overlay",
        "input_queue" => "dismiss_queue_view",
        "attachments" => "dismiss_attachment_view",
        "continuation" => "dismiss_continuation_view",
        _ => "clear_composer_or_noop",
    }
}

pub(crate) fn bottom_pane_completion_actions_json(
    active_view: &str,
    cell: &WorkbenchCell,
) -> serde_json::Value {
    match active_view {
        "approval" => serde_json::json!({
            "accept": "/screen approve-selected",
            "reject": "/screen deny-selected",
            "inspect": "/approval",
        }),
        "input_queue" => serde_json::json!({
            "accept": "/queue run",
            "reject": "/screen clear-selected",
            "inspect": "/queue",
        }),
        "attachments" => serde_json::json!({
            "accept": "/attach list",
            "reject": "/attach clear",
            "inspect": "/attach list",
        }),
        "continuation" => serde_json::json!({
            "accept": "/queue run",
            "reject": "/screen cancel-selected",
            "inspect": "/cancel",
        }),
        _ => serde_json::json!({
            "submit": "enter",
            "palette": extract_assignment_display(&cell.detail, "palette=", "/screen --palette"),
        }),
    }
}

pub(crate) fn screen_surface_progress_json(screen: &WorkbenchScreen) -> serde_json::Value {
    let Some(cell) = screen.status.iter().find(|cell| cell.title == "progress") else {
        return screen_idle_progress_json();
    };
    let kind = extract_token_after(&cell.detail, "kind=").unwrap_or_else(|| "idle".into());
    let status = extract_token_after(&cell.detail, "status=").unwrap_or_else(|| "idle".into());
    let elapsed_token = extract_token_after(&cell.detail, "elapsed_ms=");
    let elapsed_ms = elapsed_token.as_deref().and_then(|value| match value {
        "none" | "unknown" => None,
        value => value.parse::<u128>().ok(),
    });
    let phase = extract_token_after(&cell.detail, "phase=")
        .unwrap_or_else(|| progress_phase(&status).into());
    let spinner = extract_token_after(&cell.detail, "spinner=")
        .unwrap_or_else(|| progress_spinner(elapsed_ms, &status).into());
    let bar = extract_token_after(&cell.detail, "progress_bar=")
        .unwrap_or_else(|| progress_bar(&status).into());
    let error_kind =
        extract_token_after(&cell.detail, "error_kind=").filter(|value| value != "none");
    let detail = extract_assignment_span(
        &cell.detail,
        "detail=",
        &[" command=", " cancel=", " trace=", " timeline="],
    )
    .unwrap_or_else(|| "none".into());
    let is_active = matches!(status.as_str(), "running" | "queued" | "approval_pending");
    let can_interrupt = matches!(
        status.as_str(),
        "running" | "queued" | "approval_pending" | "failed"
    );
    serde_json::json!({
        "kind": kind,
        "status": status,
        "phase": phase,
        "spinner": spinner,
        "progress_bar": bar,
        "elapsed_ms": elapsed_ms,
        "error_kind": error_kind,
        "detail": detail,
        "is_active": is_active,
        "can_interrupt": can_interrupt,
    })
}

pub(crate) fn screen_idle_progress_json() -> serde_json::Value {
    serde_json::json!({
        "kind": "idle",
        "status": "idle",
        "phase": "idle",
        "spinner": "-",
        "progress_bar": "[----------]",
        "elapsed_ms": null,
        "error_kind": null,
        "detail": "none",
        "is_active": false,
        "can_interrupt": false,
    })
}

pub(crate) fn screen_modal_cell(screen: &WorkbenchScreen) -> Option<&WorkbenchCell> {
    screen.side.iter().find(|cell| {
        matches!(cell.kind, WorkbenchCellKind::Approval)
            && cell.detail.contains("approve=/approval approve ")
            && cell.detail.contains("deny=/approval deny ")
    })
}
