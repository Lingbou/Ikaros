// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn selected_cell(
    screen: &WorkbenchScreen,
    panel: WorkbenchScreenPanel,
    selection: usize,
) -> Option<&WorkbenchCell> {
    cells_for_panel(screen, panel).get(selection)
}

pub(in crate::chat::workbench::screen) fn selected_cell_index(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> Option<usize> {
    let panel = state.focused_panel();
    let cells = cells_for_panel(screen, panel);
    if let Some(selector) = state.title_selection.as_deref() {
        let selector = selector.to_ascii_lowercase();
        return cells.iter().position(|cell| {
            let title = cell.title.to_ascii_lowercase();
            title == selector || title.starts_with(&selector) || cell.kind.as_str() == selector
        });
    }
    if let Some(selector) = state.action_selection.as_deref() {
        let selector = selector.to_ascii_lowercase();
        let primary = cells.iter().position(|cell| {
            selected_cell_actions(cell)
                .first()
                .is_some_and(|command| screen_action_matches_selector(command, &selector))
        });
        return primary.or_else(|| {
            cells.iter().position(|cell| {
                selected_cell_actions(cell)
                    .iter()
                    .any(|command| screen_action_matches_selector(command, &selector))
            })
        });
    }
    let selection = state.selection_for(panel);
    cells.get(selection).map(|_| selection)
}

pub(in crate::chat::workbench::screen) fn selected_action_command(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> Option<String> {
    let selector = state.action_selection.as_deref()?.to_ascii_lowercase();
    if action_menu_global_selector(&selector) {
        if let Some(command) = selected_palette_command(state) {
            return Some(command);
        }
    }
    if let Some(command) = screen_action_menu_command_by_selector(screen, state, &selector) {
        return Some(command);
    }
    let panels = [
        state.focused_panel(),
        WorkbenchScreenPanel::Status,
        WorkbenchScreenPanel::Timeline,
        WorkbenchScreenPanel::Main,
        WorkbenchScreenPanel::Side,
    ];
    for panel in panels {
        if let Some(command) = cells_for_panel(screen, panel).iter().find_map(|cell| {
            selected_cell_actions(cell).into_iter().find(|command| {
                command != "none" && screen_action_matches_selector(command, &selector)
            })
        }) {
            return Some(command);
        }
    }
    None
}

pub(in crate::chat::workbench::screen) fn screen_action_menu_command_by_selector(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    selector: &str,
) -> Option<String> {
    if matches!(selector, "help" | "global_help" | "f1") {
        return Some("/help".into());
    }
    if matches!(
        selector,
        "palette" | "command_palette" | "global_palette" | "commands_palette" | "f5"
    ) {
        return selected_palette_command(state).or_else(|| Some("/screen --palette".into()));
    }
    let progress = screen_surface_progress_json(screen);
    let approval_overlay = screen_modal_json_value(screen);
    let input_popup = screen_input_popup_json(screen, state);
    let input_model = screen_input_model_json(screen, &input_popup);
    let active_view = find_cell(screen, |cell| cell.title == "bottom pane")
        .and_then(|cell| extract_token_after(&cell.detail, "active_view="))
        .unwrap_or_else(|| "composer".into());
    let overlay_routing =
        screen_overlay_routing_json(&active_view, &approval_overlay, &input_popup);
    let turn_state = screen_turn_state_model_json(screen, &progress, &input_model);
    let recovery = screen_recovery_model_json(screen, &turn_state);
    let action_menu = screen_action_menu_model_json(
        screen,
        state,
        &recovery,
        &approval_overlay,
        &input_popup,
        &turn_state,
        &overlay_routing,
    );
    if matches!(selector, "primary" | "default" | "action") {
        return action_menu
            .get("primary")
            .and_then(|item| item.get("command"))
            .and_then(serde_json::Value::as_str)
            .filter(|command| *command != "none")
            .map(ToOwned::to_owned);
    }
    action_menu
        .get("groups")
        .and_then(serde_json::Value::as_array)
        .and_then(|groups| {
            groups.iter().find_map(|group| {
                let group_id = group.get("id").and_then(serde_json::Value::as_str);
                group
                    .get("items")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|items| {
                        items.iter().find_map(|item| {
                            action_menu_item_command_if_matches(group_id, item, selector)
                        })
                    })
            })
        })
}

pub(in crate::chat::workbench::screen) fn action_menu_item_command_if_matches(
    group_id: Option<&str>,
    item: &serde_json::Value,
    selector: &str,
) -> Option<String> {
    let command = item
        .get("command")
        .and_then(serde_json::Value::as_str)
        .filter(|command| *command != "none")?;
    action_menu_item_matches_selector(group_id, item, selector).then(|| command.to_owned())
}

pub(in crate::chat::workbench::screen) fn action_menu_item_matches_selector(
    group_id: Option<&str>,
    item: &serde_json::Value,
    selector: &str,
) -> bool {
    let id = item.get("id").and_then(serde_json::Value::as_str);
    let label = item.get("label").and_then(serde_json::Value::as_str);
    let command = item
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    id.is_some_and(|id| action_menu_text_matches_selector(id, selector))
        || label.is_some_and(|label| action_menu_text_matches_selector(label, selector))
        || screen_action_matches_selector(command, selector)
        || group_id.is_some_and(|group_id| {
            id.is_some_and(|id| {
                let qualified = format!("{group_id}.{id}");
                action_menu_text_matches_selector(&qualified, selector)
            })
        })
}

pub(in crate::chat::workbench::screen) fn action_menu_text_matches_selector(
    value: &str,
    selector: &str,
) -> bool {
    let value = value.to_ascii_lowercase();
    let value = value.trim_start_matches('/');
    let compact = compact_action_selector(value);
    let selector = selector.trim_start_matches('/');
    let selector_compact = compact_action_selector(selector);
    value == selector
        || value.starts_with(selector)
        || compact == selector_compact
        || compact.starts_with(&selector_compact)
}

pub(in crate::chat::workbench::screen) fn compact_action_selector(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !matches!(ch, ' ' | '-' | '_' | '.'))
        .collect()
}

pub(in crate::chat::workbench::screen) fn screen_action_matches_selector(
    command: &str,
    selector: &str,
) -> bool {
    let command = command.to_ascii_lowercase();
    command == selector
        || command.starts_with(selector)
        || command.strip_prefix('/').is_some_and(|without_slash| {
            without_slash == selector || without_slash.starts_with(selector)
        })
}

pub(in crate::chat::workbench::screen) fn cells_for_panel(
    screen: &WorkbenchScreen,
    panel: WorkbenchScreenPanel,
) -> &[WorkbenchCell] {
    match panel {
        WorkbenchScreenPanel::Status => &screen.status,
        WorkbenchScreenPanel::Timeline => &screen.timeline,
        WorkbenchScreenPanel::Main => &screen.main,
        WorkbenchScreenPanel::Side => &screen.side,
    }
}

pub(in crate::chat::workbench::screen) fn parse_screen_selector(
    args: &[&str],
    start: usize,
    argument_name: &str,
) -> Result<(String, usize)> {
    let Some(first) = args.get(start) else {
        return Err(anyhow!("usage: /screen --{argument_name} <value>"));
    };
    if is_screen_selector_boundary(first) {
        return Err(anyhow!("usage: /screen --{argument_name} <value>"));
    }
    let mut parts = Vec::new();
    let mut index = start;
    while let Some(value) = args.get(index) {
        if is_screen_selector_boundary(value) {
            break;
        }
        parts.push(*value);
        index += 1;
    }
    Ok((terminal_inline(&parts.join(" ")), index))
}

pub(in crate::chat::workbench::screen) fn is_screen_selector_boundary(value: &str) -> bool {
    value.starts_with("--")
        || matches!(
            value,
            "focus"
                | "scroll"
                | "select"
                | "select-title"
                | "select-kind"
                | "select-action"
                | "tab"
                | "shift-tab"
                | "down"
                | "j"
                | "up"
                | "k"
                | "page-down"
                | "pgdn"
                | "page-up"
                | "pgup"
                | "top"
                | "home"
                | "focus-next"
                | "focus-prev"
                | "focus-previous"
                | "select-next"
                | "select-prev"
                | "select-previous"
                | "select-first"
                | "fullscreen"
                | "inline"
                | "raw"
                | "rich"
                | "palette"
                | "palette-query"
                | "close-palette"
                | "approve-selected"
                | "approve"
                | "a"
                | "deny-selected"
                | "deny"
                | "d"
                | "cancel-selected"
                | "cancel"
                | "c"
                | "clear-selected"
                | "clear"
                | "x"
                | "open-selected"
                | "open"
                | "enter"
                | "confirm-selected"
                | "confirm"
                | "help"
        )
}
