// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use super::{actions::*, input_model::*, layout::*, panels::*, render::*, selection::*};

pub(super) fn screen_panels_meta_json_value(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> Vec<serde_json::Value> {
    [
        (
            WorkbenchScreenPanel::Status,
            "status",
            "session_status",
            &screen.status,
        ),
        (
            WorkbenchScreenPanel::Timeline,
            "timeline",
            "session_timeline",
            &screen.timeline,
        ),
        (
            WorkbenchScreenPanel::Main,
            "main",
            "primary_work",
            &screen.main,
        ),
        (
            WorkbenchScreenPanel::Side,
            "side",
            "approval_queue",
            &screen.side,
        ),
    ]
    .into_iter()
    .map(|(panel, title, role, cells)| {
        let selection = state.selection_for(panel);
        let selected = selected_cell(screen, panel, selection)
            .map(screen_surface_cell_json)
            .unwrap_or(serde_json::Value::Null);
        let scroll = state.scroll_for(panel);
        serde_json::json!({
            "id": panel.as_str(),
            "title": title,
            "role": role,
            "focused": state.focused_panel() == panel,
            "scroll": scroll,
            "selection": selection.saturating_add(1),
            "cell_count": cells.len(),
            "visible_start": scroll.saturating_add(1),
            "selected": selected,
            "empty": cells.is_empty(),
        })
    })
    .collect()
}

pub(super) fn screen_footer_state_json_value(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> serde_json::Value {
    let selected_action = screen_selected_primary_action(screen, state);
    let progress = screen_surface_progress_json(screen);
    let task_running = progress
        .get("is_active")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let interruptible = progress
        .get("can_interrupt")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    serde_json::json!({
        "input_hint": terminal_inline(&screen.input_hint),
        "status": terminal_inline(&screen.footer),
        "status_line_model": screen_status_line_model_json(screen, state),
        "focused_panel": state.focused_panel().as_str(),
        "selected_action": selected_action,
        "task_running": task_running,
        "interruptible": interruptible,
        "interrupt_command": interruptible.then_some("/cancel all"),
        "open_selected_command": "/screen open-selected",
        "confirm_selected_command": "/screen confirm-selected",
        "refresh_command": "/screen",
        "key_hints": [
            {"key": "tab", "label": "focus"},
            {"key": "arrows", "label": "scroll/select"},
            {"key": "enter", "label": "open"},
            {"key": "alt+enter", "label": "confirm"},
            {"key": "alt+a", "label": "approve"},
            {"key": "alt+d", "label": "deny"},
            {"key": "alt+c", "label": "cancel"},
            {"key": "ctrl+c", "label": "interrupt"},
        ],
    })
}

pub(super) fn screen_status_line_model_json(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> serde_json::Value {
    let model = find_cell(screen, |cell| cell.title == "model");
    let session = find_cell(screen, |cell| cell.title == "session");
    let budget = find_cell(screen, |cell| cell.title == "model budget");
    let provider_health = find_cell(screen, |cell| cell.title == "provider health");
    let provider_recovery = find_cell(screen, |cell| cell.title == "provider recovery");
    let queue = find_cell(screen, |cell| cell.title == "queue");
    let bottom = find_cell(screen, |cell| cell.title == "bottom pane");
    let progress = find_cell(screen, |cell| cell.title == "progress");
    let gateway = find_cell(screen, |cell| cell.title == "gateway");
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
    let progress_status = progress
        .and_then(|cell| extract_token_after(&cell.detail, "status="))
        .unwrap_or_else(|| "idle".into());
    let action_required = approval_count != "0"
        || failed != "0"
        || progress_status == "approval_pending"
        || progress_status == "failed"
        || budget_status == "exhausted"
        || provider_status.contains("Cooldown")
        || provider_status.contains("Unhealthy");
    let selected_action = screen_selected_primary_action(screen, state);
    serde_json::json!({
        "schema": "ikaros-workbench-status-line-v1",
        "version": 1,
        "action_required": action_required,
        "segments": [
            {
                "id": "activity",
                "label": "activity",
                "value": progress_status.clone(),
                "attention": progress_status == "approval_pending" || progress_status == "failed",
                "command": "/trace",
            },
            {
                "id": "model",
                "label": "model",
                "value": format!(
                    "{}/{}",
                    model
                        .and_then(|cell| extract_token_after(&cell.detail, "provider="))
                        .unwrap_or_else(|| "unknown".into()),
                    model
                        .and_then(|cell| extract_token_after(&cell.detail, "model="))
                        .unwrap_or_else(|| "unknown".into()),
                ),
                "attention": provider_status.contains("Cooldown") || provider_status.contains("Unhealthy"),
                "command": "/provider",
            },
            {
                "id": "budget",
                "label": "budget",
                "value": budget_status.clone(),
                "attention": matches!(budget_status.as_str(), "near_limit" | "exhausted"),
                "command": "/budget",
            },
            {
                "id": "approval",
                "label": "approval",
                "value": approval_count.clone(),
                "attention": approval_count != "0",
                "command": "/approval",
            },
            {
                "id": "queue",
                "label": "queue",
                "value": format!("q:{queued} r:{running} f:{failed}"),
                "attention": running != "0" || failed != "0",
                "command": "/debug continuations",
            },
            {
                "id": "session",
                "label": "session",
                "value": session
                    .and_then(|cell| extract_token_after(&cell.detail, "id="))
                    .unwrap_or_else(|| "unknown".into()),
                "attention": false,
                "command": "/session",
            },
            {
                "id": "gateway",
                "label": "gateway",
                "value": gateway
                    .and_then(|cell| extract_token_after(&cell.detail, "status="))
                    .unwrap_or_else(|| "unknown".into()),
                "attention": false,
                "command": "/gateway",
            },
        ],
        "selected_action": selected_action,
        "actions": {
            "configure": "/screen",
            "provider": "/provider",
            "budget": "/budget",
            "approval": "/approval",
            "queue": "/debug continuations",
            "trace": "/trace",
        },
    })
}

pub(super) fn screen_state_trace_json_value(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    selected: &serde_json::Value,
    surface: &serde_json::Value,
) -> Vec<serde_json::Value> {
    let progress = surface
        .get("progress")
        .cloned()
        .unwrap_or_else(screen_idle_progress_json);
    let bottom_pane = surface
        .get("bottom_pane")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"active_view": "composer"}));
    let modal = surface
        .get("modal")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let overlay_routing = surface
        .get("overlay_routing")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"active_overlay": "composer"}));
    vec![
        serde_json::json!({
            "phase": "screen_render",
            "source": "workbench",
            "focused_panel": state.focused_panel().as_str(),
            "fullscreen": state.fullscreen(),
        }),
        serde_json::json!({
            "phase": "panel_selection",
            "source": "workbench",
            "selected": selected,
        }),
        serde_json::json!({
            "phase": "bottom_pane",
            "source": "workbench",
            "active_view": bottom_pane.get("active_view").cloned().unwrap_or_else(|| serde_json::json!("composer")),
            "modal_stack": bottom_pane.get("modal_stack").cloned().unwrap_or_else(|| serde_json::json!([])),
        }),
        serde_json::json!({
            "phase": "progress",
            "source": "workbench",
            "progress": progress,
        }),
        serde_json::json!({
            "phase": "modal",
            "source": "workbench",
            "kind": modal,
            "visible": modal != "none",
            "approval_count": screen.side.iter().filter(|cell| matches!(cell.kind, WorkbenchCellKind::Approval)).count(),
        }),
        serde_json::json!({
            "phase": "input_routing",
            "source": "workbench",
            "active_overlay": overlay_routing
                .get("active_overlay")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("composer")),
            "modal_scope": overlay_routing
                .get("modal_scope")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("none")),
            "enter_target": overlay_routing
                .get("enter_target")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("submit_input")),
            "esc_target": overlay_routing
                .get("esc_target")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("cancel_input")),
            "captures_text_input": overlay_routing
                .get("captures_text_input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(false)),
        }),
    ]
}

pub(super) fn screen_surface_json_value(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> serde_json::Value {
    let active_cell = screen
        .main
        .iter()
        .find(|cell| cell.title.starts_with("active "))
        .map(screen_surface_cell_json)
        .unwrap_or(serde_json::Value::Null);
    let has_active_cell = !active_cell.is_null();
    let bottom_pane = screen
        .status
        .iter()
        .find(|cell| cell.title == "bottom pane")
        .map(screen_bottom_pane_json)
        .unwrap_or_else(|| {
            serde_json::json!({
                "active_view": "composer",
                "modal_stack": [],
                "approvals": "0",
                "pending_inputs": "0",
                "attachments": "0",
                "continuations": "0",
                "next_input": "none",
                "composer": {
                    "input": "readline",
                    "completion": "complete",
                    "history": "history",
                    "palette": "/screen --palette",
                },
            })
        });
    let progress = screen_surface_progress_json(screen);
    let approval_overlay = screen_modal_json_value(screen);
    let input_popup = screen_input_popup_json(screen, state);
    let input_model = screen_input_model_json(screen, &input_popup);
    let active_view = bottom_pane
        .get("active_view")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("composer");
    let overlay_routing = screen_overlay_routing_json(active_view, &approval_overlay, &input_popup);
    let turn_state_model = screen_turn_state_model_json(screen, &progress, &input_model);
    let recovery_model = screen_recovery_model_json(screen, &turn_state_model);
    let action_menu_model = screen_action_menu_model_json(
        screen,
        state,
        &recovery_model,
        &approval_overlay,
        &input_popup,
        &turn_state_model,
        &overlay_routing,
    );
    let bottom_pane_model = screen_bottom_pane_model_json(
        &bottom_pane,
        &approval_overlay,
        &input_popup,
        &input_model,
        &turn_state_model,
        &recovery_model,
        &overlay_routing,
    );
    let transcript_model = screen_transcript_model_json(screen, state, &active_cell);
    let modal_kind = json_string(&overlay_routing, "modal_scope", "none");
    serde_json::json!({
        "schema": "ikaros-workbench-surface-v1",
        "version": 1,
        "layout": "transcript_active_bottom_pane",
        "focused_panel": state.focused_panel().as_str(),
        "modal": modal_kind,
        "transcript": {
            "committed_timeline_cells": screen.timeline.len(),
            "committed_main_cells": screen
                .main
                .iter()
                .filter(|cell| !cell.title.starts_with("active "))
                .count(),
            "has_live_tail": has_active_cell,
            "live_tail": active_cell.clone(),
        },
        "transcript_model": transcript_model,
        "active_cell": active_cell,
        "bottom_pane": bottom_pane,
        "bottom_pane_model": bottom_pane_model,
        "action_menu_model": action_menu_model,
        "approval_overlay": approval_overlay,
        "status_surfaces": screen_status_surfaces_json(
            &bottom_pane,
            &approval_overlay,
            &progress,
            &input_popup,
            &input_model,
            &overlay_routing,
        ),
        "input_popup": input_popup,
        "input_model": input_model,
        "overlay_routing": overlay_routing,
        "status_line_model": screen_status_line_model_json(screen, state),
        "navigation": screen_navigation_json(),
        "timeline_panel": screen_timeline_panel_json(screen),
        "timeline_tabs_model": screen_timeline_tabs_model_json(screen),
        "timeline_groups": screen_timeline_groups_json(screen),
        "timeline_group_model": screen_timeline_group_model_json(screen),
        "coding_groups": screen_coding_groups_json(screen),
        "coding_loop_model": screen_coding_loop_model_json(screen),
        "dashboard_model": screen_dashboard_model_json(screen),
        "keymap_model": screen_keymap_model_json(screen, state),
        "evidence": screen_evidence_json(screen),
        "surface_index": screen_surface_index_json(screen),
        "provider_panel": screen_provider_panel_json(screen),
        "context_panel": screen_context_panel_json(screen),
        "memory_panel": screen_memory_panel_json(screen),
        "rag_panel": screen_rag_panel_json(screen),
        "coding_panel": screen_coding_panel_json(screen),
        "approval_panel": screen_approval_panel_json(screen),
        "approval_decision_model": screen_approval_decision_model_json(screen),
        "queue_panel": screen_queue_panel_json(screen),
        "progress": progress,
        "turn_state_model": turn_state_model,
        "recovery_model": recovery_model,
        "readiness_model": screen_readiness_model_json(screen),
        "debug_model": screen_debug_model_json(screen),
    })
}

pub(super) fn screen_surface_index_json(screen: &WorkbenchScreen) -> Vec<serde_json::Value> {
    [
        (
            "provider",
            WorkbenchCellKind::Model,
            "/screen --focus main --select-action provider",
            "/provider",
        ),
        (
            "context",
            WorkbenchCellKind::Context,
            "/screen --focus main --select-action context",
            "/context",
        ),
        (
            "memory",
            WorkbenchCellKind::Memory,
            "/screen --focus main --select-action memory",
            "/memory",
        ),
        (
            "rag",
            WorkbenchCellKind::Context,
            "/screen --focus main --select-action rag",
            "/rag",
        ),
        (
            "coding",
            WorkbenchCellKind::Coding,
            "/screen --focus main --select-action code",
            "/code plan",
        ),
        (
            "approval",
            WorkbenchCellKind::Approval,
            "/screen --focus side --select-action approval",
            "/approval",
        ),
        (
            "queue",
            WorkbenchCellKind::Continuation,
            "/screen --focus side --select-action queue",
            "/debug continuations",
        ),
        (
            "gateway",
            WorkbenchCellKind::Session,
            "/screen --focus status --select-action gateway",
            "/gateway",
        ),
    ]
    .into_iter()
    .map(|(surface, fallback_kind, focus, open)| {
        let evidence = screen_evidence_area_json(screen, surface, fallback_kind);
        let cell_count = evidence
            .get("cell_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let attention = evidence
            .get("attention")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        serde_json::json!({
            "surface": surface,
            "visible": cell_count > 0,
            "attention": attention,
            "cell_count": cell_count,
            "focus": focus,
            "open": open,
            "primary": if attention { focus } else { open },
            "evidence": evidence,
        })
    })
    .collect()
}

pub(super) fn screen_status_surfaces_json(
    bottom_pane: &serde_json::Value,
    approval_overlay: &serde_json::Value,
    progress: &serde_json::Value,
    input_popup: &serde_json::Value,
    input_model: &serde_json::Value,
    overlay_routing: &serde_json::Value,
) -> serde_json::Value {
    let active_view = bottom_pane
        .get("active_view")
        .and_then(|value| value.as_str())
        .unwrap_or("composer");
    let modal_stack = bottom_pane
        .get("modal_stack")
        .cloned()
        .unwrap_or_else(|| serde_json::json!([]));
    let pending_inputs = bottom_pane
        .get("pending_inputs")
        .and_then(|value| value.as_str())
        .unwrap_or("0");
    let attachments = bottom_pane
        .get("attachments")
        .and_then(|value| value.as_str())
        .unwrap_or("0");
    let continuations = bottom_pane
        .get("continuations")
        .and_then(|value| value.as_str())
        .unwrap_or("0");
    let progress_status = progress
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or("idle");
    let can_interrupt = progress
        .get("can_interrupt")
        .and_then(|value| value.as_bool())
        .unwrap_or(false);
    let approval_visible = !approval_overlay.is_null();
    let input_popup_visible = !input_popup.is_null();
    let active_overlay = json_string(overlay_routing, "active_overlay", "composer");
    let modal_scope = json_string(overlay_routing, "modal_scope", "none");

    serde_json::json!({
        "schema": "ikaros-workbench-status-surfaces-v1",
        "active_view": active_view.clone(),
        "active_overlay": active_overlay,
        "modal_scope": modal_scope,
        "overlay_routing": overlay_routing.clone(),
        "modal_stack": modal_stack,
        "composer": {
            "retained": true,
            "visible": active_view == "composer",
            "popup": input_popup.clone(),
            "input_model": input_model.clone(),
            "input": bottom_pane
                .get("composer")
                .and_then(|value| value.get("input"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!("readline")),
            "completion": bottom_pane
                .get("composer")
                .and_then(|value| value.get("completion"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!("complete")),
            "history": bottom_pane
                .get("composer")
                .and_then(|value| value.get("history"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!("history")),
            "palette": bottom_pane
                .get("composer")
                .and_then(|value| value.get("palette"))
                .cloned()
                .unwrap_or_else(|| serde_json::json!("/screen --palette")),
        },
        "approval": {
            "visible": approval_visible,
            "count": bottom_pane
                .get("approvals")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("0")),
            "primary_id": approval_overlay
                .get("primary_id")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            "approve": approval_overlay
                .get("approve")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("/screen approve-selected")),
            "deny": approval_overlay
                .get("deny")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("/screen deny-selected")),
            "inspect": approval_overlay
                .get("inspect")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("/approval list")),
        },
        "pending_input_preview": {
            "visible": active_view == "input_queue" || pending_inputs != "0",
            "count": pending_inputs,
            "next_input": bottom_pane
                .get("next_input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("none")),
            "run": "/queue run",
            "clear": "/queue clear",
        },
        "command_palette": {
            "visible": input_popup
                .get("kind")
                .and_then(|value| value.as_str())
                .is_some_and(|kind| kind == "command_completion" || kind == "command_palette"),
            "active": overlay_routing
                .get("active_overlay")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "command_palette"),
            "captures_text_input": overlay_routing
                .get("text_target")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|target| target == "command_palette_filter"),
            "popup": if input_popup_visible {
                input_popup.clone()
            } else {
                serde_json::Value::Null
            },
        },
        "history_search": {
            "visible": input_popup
                .get("kind")
                .and_then(|value| value.as_str())
                .is_some_and(|kind| kind == "history_search"),
            "active": overlay_routing
                .get("active_overlay")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|kind| kind == "history_search"),
            "popup": if input_popup_visible {
                input_popup.clone()
            } else {
                serde_json::Value::Null
            },
        },
        "attachments": {
            "visible": active_view == "attachments" || attachments != "0",
            "count": attachments,
            "list": "/attach list",
            "clear": "/attach clear",
        },
        "continuations": {
            "visible": active_view == "continuation" || continuations != "0",
            "count": continuations,
            "debug": "/debug continuations",
            "run": "/queue run",
            "cancel": "/cancel all",
        },
        "unified_exec_footer": {
            "status": progress_status,
            "phase": progress
                .get("phase")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("idle")),
            "spinner": progress
                .get("spinner")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("-")),
            "progress_bar": progress
                .get("progress_bar")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("[----------]")),
            "can_interrupt": can_interrupt,
            "interrupt": if can_interrupt {
                serde_json::json!("/cancel all")
            } else {
                serde_json::Value::Null
            },
        },
        "status_line": {
            "left": ["Tab complete", "Ctrl-R history", "Ctrl-Z undo", "Ctrl-Y redo"],
            "right": ["Alt-A approve", "Alt-D deny", "Alt-C cancel", "Enter open", "Alt-Enter confirm"],
        },
    })
}
