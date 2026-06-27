// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn screen_bottom_pane_model_json(
    bottom_pane: &serde_json::Value,
    approval_overlay: &serde_json::Value,
    input_popup: &serde_json::Value,
    input_model: &serde_json::Value,
    turn_state: &serde_json::Value,
    recovery_model: &serde_json::Value,
    overlay_routing: &serde_json::Value,
) -> serde_json::Value {
    let active_view = bottom_pane
        .get("active_view")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("composer");
    let active_surface = json_string(overlay_routing, "active_overlay", active_view);
    let view_stack = bottom_pane
        .get("modal_stack")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let enter_target = json_string(overlay_routing, "enter_target", "submit_input");
    let alt_enter_target = json_string(
        overlay_routing,
        "alt_enter_target",
        "insert_newline_or_confirm_selected",
    );
    let esc_target = json_string(overlay_routing, "esc_target", "cancel_input");
    let ctrl_c_target = match json_string(
        overlay_routing,
        "ctrl_c_target",
        "exit_or_clear_or_interrupt",
    )
    .as_str()
    {
        "/cancel all" => "exit_or_clear_or_interrupt".into(),
        target => target.to_owned(),
    };
    let tab_target = json_string(overlay_routing, "tab_target", "/screen --focus-next");
    let mut overlay_routing_model = overlay_routing.clone();
    if let Some(routing) = overlay_routing_model.as_object_mut() {
        routing.insert(
            "ctrl_c_target".into(),
            serde_json::Value::String(ctrl_c_target.clone()),
        );
    }

    serde_json::json!({
        "schema": "ikaros-workbench-bottom-pane-v1",
        "active_view": active_view,
        "active_surface": active_surface,
        "overlay_routing": overlay_routing_model,
        "view_stack": view_stack,
        "composer_retained": true,
        "composer": input_model.clone(),
        "input_popup": input_popup.clone(),
        "approval_overlay": approval_overlay.clone(),
        "turn_state": turn_state.clone(),
        "recovery": recovery_model.clone(),
        "routing": {
            "active_overlay": overlay_routing
                .get("active_overlay")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("composer")),
            "modal_scope": overlay_routing
                .get("modal_scope")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("none")),
            "captures_text_input": overlay_routing
                .get("captures_text_input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(false)),
            "text_target": overlay_routing
                .get("text_target")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("none")),
            "enter": enter_target,
            "alt_enter": alt_enter_target,
            "esc": esc_target,
            "ctrl_c": ctrl_c_target,
            "tab": tab_target,
        },
        "actions": {
            "palette": "/screen --palette",
            "history_search": "ctrl-r",
            "approve": "/screen approve-selected",
            "deny": "/screen deny-selected",
            "cancel": "/cancel all",
            "recover": recovery_model
                .get("primary")
                .and_then(|value| value.get("command"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!("none")),
        },
    })
}

pub(crate) fn screen_turn_state_model_json(
    screen: &WorkbenchScreen,
    progress: &serde_json::Value,
    input_model: &serde_json::Value,
) -> serde_json::Value {
    let bottom = find_cell(screen, |cell| cell.title == "bottom pane");
    let queue = find_cell(screen, |cell| cell.title == "queue");
    let budget = find_cell(screen, |cell| cell.title == "model budget");
    let provider_health = find_cell(screen, |cell| cell.title == "provider health");
    let provider_recovery = find_cell(screen, |cell| cell.title == "provider recovery");
    let bottom_approval_count = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "approvals="))
        .unwrap_or_else(|| "0".into());
    let side_approval_count = screen
        .side
        .iter()
        .filter(|cell| {
            matches!(cell.kind, WorkbenchCellKind::Approval)
                && (cell.title.starts_with("pending ")
                    || (cell.detail.contains("approve=/approval approve ")
                        && cell.detail.contains("deny=/approval deny ")))
        })
        .count();
    let approval_count = if bottom_approval_count == "0" && side_approval_count > 0 {
        side_approval_count.to_string()
    } else {
        bottom_approval_count
    };
    let pending_inputs = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "pending_inputs="))
        .unwrap_or_else(|| "0".into());
    let queued = queue
        .and_then(|cell| extract_token_after(&cell.detail, "queued="))
        .unwrap_or_else(|| "0".into());
    let running = queue
        .and_then(|cell| extract_token_after(&cell.detail, "running="))
        .unwrap_or_else(|| "0".into());
    let failed = queue
        .and_then(|cell| extract_token_after(&cell.detail, "failed="))
        .unwrap_or_else(|| "0".into());
    let budget_status = budget
        .and_then(|cell| extract_token_after(&cell.detail, "budget_status="))
        .unwrap_or_else(|| "unknown".into());
    let provider_status = provider_health
        .and_then(|cell| extract_token_after(&cell.detail, "health_status="))
        .or_else(|| provider_recovery.and_then(|cell| extract_token_after(&cell.detail, "status=")))
        .unwrap_or_else(|| "unknown".into());
    let progress_status = json_string(progress, "status", "idle");
    let error_kind = json_string(progress, "error_kind", "none");
    let input_dirty = input_model
        .get("dirty")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let input_blocked = input_model
        .get("blocks_input")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let state = if approval_count != "0" || progress_status == "approval_pending" {
        "approval_pending"
    } else if progress_status == "running" || running != "0" {
        "running"
    } else if budget_status == "exhausted" || error_kind == "budget_exceeded" {
        "provider_limited"
    } else if failed != "0" || progress_status == "failed" {
        "failed"
    } else if queued != "0" || pending_inputs != "0" {
        "queued"
    } else if input_blocked {
        "input_blocked"
    } else if input_dirty {
        "composing"
    } else {
        "idle"
    };
    let blocking_reason = match state {
        "approval_pending" => "approval_required",
        "running" => "turn_running",
        "provider_limited" => "model_budget_exhausted",
        "failed" => "failure_requires_recovery",
        "queued" => "queued_input_or_continuation",
        "input_blocked" => "bottom_pane_overlay",
        _ => "none",
    };
    let can_cancel = matches!(state, "running" | "approval_pending" | "queued" | "failed");
    let can_resume = matches!(state, "queued" | "provider_limited" | "failed");
    let primary_command = match state {
        "approval_pending" => "/approval",
        "running" => "/cancel all",
        "provider_limited" => "/budget",
        "failed" => "/trace --failed",
        "queued" => "/queue run",
        "input_blocked" => "dismiss_active_surface",
        "composing" => "enter",
        _ => "none",
    };
    let primary_action = action_menu_item_json(
        "turn_primary",
        command_action_label(primary_command),
        primary_command,
        command_shortcut(primary_command),
        command_requires_explicit_action(primary_command),
    );

    serde_json::json!({
        "schema": "ikaros-turn-state-v1",
        "state": state,
        "progress_status": progress_status,
        "error_kind": error_kind,
        "blocking_reason": blocking_reason,
        "can_submit": !input_blocked && (state == "idle" || state == "composing"),
        "can_cancel": can_cancel,
        "can_resume": can_resume,
        "input_dirty": input_dirty,
        "input_blocked": input_blocked,
        "primary": primary_action,
        "interrupt": {
            "available": can_cancel,
            "cancel": "/cancel all",
            "trace": "/trace",
            "timeline": "/timeline",
            "shortcut": "ctrl-c/alt-i",
        },
        "counts": {
            "approvals": approval_count,
            "pending_inputs": pending_inputs,
            "queued_continuations": queued,
            "running_continuations": running,
            "failed_continuations": failed,
        },
        "provider": {
            "budget_status": budget_status,
            "health_status": provider_status,
        },
        "actions": {
            "submit": "enter",
            "cancel": "/cancel all",
            "approve": "/screen approve-selected",
            "deny": "/screen deny-selected",
            "resume_queue": "/queue run",
            "budget": "/budget",
            "provider": "/provider health",
            "trace": "/trace",
            "timeline": "/timeline",
        },
    })
}
