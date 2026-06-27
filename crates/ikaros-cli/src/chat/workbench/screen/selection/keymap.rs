// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn screen_key_bindings_json() -> Vec<serde_json::Value> {
    [
        ("tab", "focus_next", "/screen --focus-next"),
        ("shift-tab", "focus_previous", "/screen --focus-prev"),
        ("down", "scroll_down", "/screen --down"),
        ("up", "scroll_up", "/screen --up"),
        ("page-down", "page_down", "/screen --page-down"),
        ("page-up", "page_up", "/screen --page-up"),
        ("home", "top", "/screen --top"),
        ("right", "select_next", "/screen --select-next"),
        ("left", "select_previous", "/screen --select-prev"),
        ("alt+j", "scroll_down", "/screen --down"),
        ("alt+k", "scroll_up", "/screen --up"),
        ("alt+l", "select_next", "/screen --select-next"),
        ("alt+h", "select_previous", "/screen --select-prev"),
        ("enter", "open_selected", "/screen open-selected"),
        ("alt+a", "approve_selected", "/screen approve-selected"),
        ("alt+d", "deny_selected", "/screen deny-selected"),
        ("alt+c", "cancel_selected", "/screen cancel-selected"),
        ("alt+x", "clear_selected", "/screen clear-selected"),
        ("alt-enter", "confirm_selected", "/screen confirm-selected"),
        ("command", "raw_mode", "/screen --raw"),
        ("command", "rich_mode", "/screen --rich"),
    ]
    .into_iter()
    .map(|(key, action, command)| {
        serde_json::json!({
            "key": key,
            "action": action,
            "intent": command_intent(command),
            "scope": command_scope(command),
            "risk": command_risk(command),
            "command": command,
        })
    })
    .collect()
}

pub(in crate::chat::workbench::screen) fn screen_keymap_model_json(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> serde_json::Value {
    let bottom = find_cell(screen, |cell| cell.title == "bottom pane");
    let active_view = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "active_view="))
        .unwrap_or_else(|| "composer".into());
    let popup = screen_input_popup_json(screen, state);
    let popup_kind = popup
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let focused_panel = state.focused_panel().as_str();
    let approval_overlay = screen_modal_json_value(screen);
    let overlay_routing = screen_overlay_routing_json(&active_view, &approval_overlay, &popup);
    let active_scope = json_string(&overlay_routing, "active_scope", "composer");
    let modal_scope = json_string(&overlay_routing, "modal_scope", "none");

    serde_json::json!({
        "schema": "ikaros-workbench-keymap-v1",
        "focused_panel": focused_panel,
        "active_scope": active_scope,
        "modal_scope": modal_scope,
        "groups": [
            keymap_group_json(
                "global",
                "Global",
                vec![
                    keymap_binding_json("tab", "focus_next", "/screen --focus-next", "global", "always", "Move focus to the next panel"),
                    keymap_binding_json("shift-tab", "focus_previous", "/screen --focus-prev", "global", "always", "Move focus to the previous panel"),
                    keymap_binding_json("alt+1", "focus_status", "/screen --focus status", "global", "always", "Focus the status/header panel"),
                    keymap_binding_json("alt+2", "focus_timeline", "/screen --focus timeline", "global", "always", "Focus the timeline/replay panel"),
                    keymap_binding_json("alt+3", "focus_main", "/screen --focus main", "global", "always", "Focus the main evidence panel"),
                    keymap_binding_json("alt+4", "focus_side", "/screen --focus side", "global", "always", "Focus approvals, queue, and attachments"),
                    keymap_binding_json("ctrl-c", "exit_or_clear_or_interrupt", "stateful", "global", "overlay_or_composer_or_task", "Close overlays, clear input, interrupt active work, or exit when idle"),
                    keymap_binding_json("ctrl-l", "refresh", "/screen", "global", "always", "Refresh the current workbench frame"),
                    keymap_binding_json("f1", "select_help", "/help", "global", "always", "Select workbench help"),
                    keymap_binding_json("f5", "open_command_palette", "/screen --palette", "global", "always", "Open the command palette overlay"),
                ],
            ),
            keymap_group_json(
                "panel_navigation",
                "Panel Navigation",
                vec![
                    keymap_binding_json("up", "scroll_up", "/screen --up", focused_panel, "composer_empty", "Scroll focused panel up"),
                    keymap_binding_json("down", "scroll_down", "/screen --down", focused_panel, "composer_empty", "Scroll focused panel down"),
                    keymap_binding_json("page-up", "page_up", "/screen --page-up", focused_panel, "composer_empty", "Page focused panel up"),
                    keymap_binding_json("page-down", "page_down", "/screen --page-down", focused_panel, "composer_empty", "Page focused panel down"),
                    keymap_binding_json("home", "top", "/screen --top", focused_panel, "composer_empty", "Jump to the top of focused panel"),
                    keymap_binding_json("left", "select_previous", "/screen --select-prev", focused_panel, "composer_empty", "Select previous action/cell"),
                    keymap_binding_json("right", "select_next", "/screen --select-next", focused_panel, "composer_empty", "Select next action/cell"),
                    keymap_binding_json("enter", "open_selected", "/screen open-selected", focused_panel, "composer_empty", "Open selected cell action"),
                    keymap_binding_json("alt-enter", "confirm_selected", "/screen confirm-selected", focused_panel, "composer_empty", "Confirm explicit selected action"),
                ],
            ),
            keymap_group_json(
                "composer",
                "Composer",
                vec![
                    keymap_binding_json("enter", "submit", "submit_input", "composer", "composer_dirty", "Submit current input"),
                    keymap_binding_json("alt-enter", "newline", "insert_newline", "composer", "composer_active", "Insert a newline"),
                    keymap_binding_json("tab", "complete", "complete_command", "composer", "slash_command", "Complete or cycle slash commands"),
                    keymap_binding_json("ctrl-r", "history_search_previous", "history_search_previous", "composer", "composer_active", "Open or cycle reverse history search"),
                    keymap_binding_json("ctrl-s", "history_search_next", "history_search_next", "composer", "history_search", "Cycle history search forward"),
                    keymap_binding_json("ctrl-z", "undo", "undo_input", "composer", "composer_active", "Undo input edit"),
                    keymap_binding_json("ctrl-y", "redo", "redo_input", "composer", "composer_active", "Redo input edit"),
                    keymap_binding_json("alt-b", "move_word_left", "move_word_left", "composer", "composer_active", "Move one word left"),
                    keymap_binding_json("alt-f", "move_word_right", "move_word_right", "composer", "composer_active", "Move one word right"),
                    keymap_binding_json("ctrl-w", "delete_previous_word", "delete_previous_word", "composer", "composer_active", "Delete previous word"),
                    keymap_binding_json("alt-d", "delete_next_word", "delete_next_word", "composer", "composer_active", "Delete next word"),
                    keymap_binding_json("esc", "clear_selection_or_cancel_input_surface", "clear_selection_or_cancel_input_surface", "composer", "history_search_or_overlay", "Clear selected action/history search or cancel active input surface"),
                ],
            ),
            keymap_group_json(
                "command_palette",
                "Command Palette",
                vec![
                    keymap_binding_json("up", "palette_previous", "/screen --palette --up", "command_palette", "palette_open", "Move to the previous command"),
                    keymap_binding_json("down", "palette_next", "/screen --palette --down", "command_palette", "palette_open", "Move to the next command"),
                    keymap_binding_json("page-up", "palette_page_previous", "/screen --palette --page-up", "command_palette", "palette_open", "Move up one page in the command palette"),
                    keymap_binding_json("page-down", "palette_page_next", "/screen --palette --page-down", "command_palette", "palette_open", "Move down one page in the command palette"),
                    keymap_binding_json("home", "palette_first", "/screen --palette --top", "command_palette", "palette_open", "Jump to the first command"),
                    keymap_binding_json("tab", "palette_cycle", "/screen --palette --down", "command_palette", "palette_open", "Cycle command palette selection"),
                    keymap_binding_json("text", "palette_type_filter", "type_palette_filter", "command_palette", "palette_open", "Type to filter slash commands"),
                    keymap_binding_json("backspace", "palette_backspace_filter", "backspace_palette_filter", "command_palette", "palette_open", "Delete the last command-palette filter character"),
                    keymap_binding_json("ctrl-u", "palette_clear_filter", "clear_palette_filter", "command_palette", "palette_open", "Clear the command-palette filter"),
                    keymap_binding_json("enter", "palette_accept", "/screen open-selected", "command_palette", "palette_open", "Open selected command"),
                    keymap_binding_json("alt-enter", "palette_confirm", "/screen confirm-selected", "command_palette", "palette_open", "Confirm selected command"),
                    keymap_binding_json("esc", "palette_close", "/screen --close-palette", "command_palette", "palette_open", "Close the command palette"),
                ],
            ),
            keymap_group_json(
                "approval",
                "Approval",
                vec![
                    keymap_binding_json("alt+a", "approve_selected", "/screen approve-selected", "approval", "approval_pending", "Approve selected approval request"),
                    keymap_binding_json("alt+d", "deny_selected", "/screen deny-selected", "approval", "approval_pending", "Deny selected approval request"),
                    keymap_binding_json("enter", "inspect_approval", "/approval", "approval", "approval_pending", "Inspect selected approval request"),
                ],
            ),
            keymap_group_json(
                "queue",
                "Queue",
                vec![
                    keymap_binding_json("alt+c", "cancel_selected", "/screen cancel-selected", "queue", "continuation_or_active_work", "Cancel selected continuation or active work"),
                    keymap_binding_json("alt+x", "clear_selected", "/screen clear-selected", "queue", "pending_input_or_attachment", "Clear selected pending input or attachment"),
                    keymap_binding_json("enter", "run_queue", "/queue run", "queue", "pending_input", "Run queued input"),
                ],
            ),
            keymap_group_json(
                "action_menu",
                "Action Menu",
                vec![
                    keymap_binding_json("enter", "open_primary_action", "/screen --select-action primary open-selected", "action_menu", "action_available", "Open the current action-menu primary item"),
                    keymap_binding_json("alt-enter", "confirm_primary_action", "/screen --select-action primary confirm-selected", "action_menu", "explicit_action_available", "Confirm the current action-menu primary item"),
                    keymap_binding_json("alt+m", "select_primary", "/screen --select-action primary", "action_menu", "action_available", "Select the current action-menu primary item"),
                    keymap_binding_json("alt+r", "select_recovery", "/screen --select-action recovery_primary", "action_menu", "recovery_available", "Select the recovery primary action"),
                    keymap_binding_json("alt+o", "select_approval", "/screen --select-action approval_approve", "action_menu", "approval_pending", "Select the approve option from the approval action menu"),
                    keymap_binding_json("alt+q", "select_queue_cancel", "/screen --select-action queue_cancel_all", "action_menu", "queue_attention", "Select queue cancellation from the action menu"),
                    keymap_binding_json("alt+i", "select_interrupt", "/screen --select-action interrupt_cancel", "action_menu", "active_work", "Select active turn cancellation"),
                ],
            ),
            keymap_group_json(
                "timeline_tabs",
                "Timeline Tabs",
                vec![
                    keymap_binding_json("ctrl-t", "timeline_all", "/screen --select-action timeline_all", "timeline", "always", "Select the all-events timeline tab"),
                ],
            ),
            keymap_group_json(
                "render_mode",
                "Render Mode",
                vec![
                    keymap_binding_json("f2", "toggle_render_mode", "toggle_render_mode", "screen", "always", "Toggle raw/rich cell rendering"),
                    keymap_binding_json("f3", "raw_mode", "/screen --raw", "screen", "always", "Switch to raw cell rendering"),
                    keymap_binding_json("f4", "rich_mode", "/screen --rich", "screen", "always", "Switch to rich markdown rendering"),
                ],
            ),
        ],
        "routing": {
            "priority": overlay_routing
                .get("priority")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(["approval", "command_palette", "history_search", "command_completion", "queue", "composer", "panel_navigation", "global"])),
            "empty_composer_panel_navigation": true,
            "approval_overlay_captures_approval_keys": screen_modal_cell(screen).is_some(),
            "active_bottom_pane_view": active_view,
            "active_popup": popup_kind,
            "active_overlay": overlay_routing
                .get("active_overlay")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("composer")),
            "captures_text_input": overlay_routing
                .get("captures_text_input")
                .cloned()
                .unwrap_or_else(|| serde_json::json!(false)),
            "text_target": overlay_routing
                .get("text_target")
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
        },
    })
}

pub(in crate::chat::workbench::screen) fn keymap_group_json(
    id: &str,
    label: &str,
    bindings: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "label": label,
        "bindings": bindings,
    })
}

pub(in crate::chat::workbench::screen) fn keymap_binding_json(
    key: &str,
    action: &str,
    command: &str,
    scope: &str,
    when: &str,
    description: &str,
) -> serde_json::Value {
    serde_json::json!({
        "key": key,
        "action": action,
        "command": command,
        "scope": scope,
        "when": when,
        "description": description,
        "intent": command_intent(command),
        "risk": command_risk(command),
        "requires_explicit_action": command_requires_explicit_action(command),
    })
}
