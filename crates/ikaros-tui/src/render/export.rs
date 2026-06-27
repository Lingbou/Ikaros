// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub fn screen_selected_cell_line(screen: &WorkbenchScreen, state: &WorkbenchScreenState) -> String {
    let panel = state.focused_panel();
    let Some(selection) = selected_cell_index(screen, state) else {
        return format!(
            "screen_selected: panel={} row={} kind=none title=none detail=none",
            panel.as_str(),
            state.selection_for(panel).saturating_add(1)
        );
    };
    let Some(cell) = selected_cell(screen, panel, selection) else {
        return format!(
            "screen_selected: panel={} row={} kind=none title=none detail=none",
            panel.as_str(),
            selection.saturating_add(1)
        );
    };
    format!(
        "screen_selected: panel={} row={} kind={} title={} detail={}",
        panel.as_str(),
        selection.saturating_add(1),
        cell.kind.as_str(),
        terminal_inline(&cell.title),
        terminal_inline(&render_cell_detail_summary(&cell.detail)),
    )
}

pub(crate) fn selected_cell_detail_line(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> String {
    let panel = state.focused_panel();
    let row = selected_cell_index(screen, state).unwrap_or_else(|| state.selection_for(panel));
    let scroll = state.scroll_for(panel);
    if row < scroll {
        return format!(
            "selected panel={} row=none kind=none action=none detail=selection_above_visible_scroll",
            panel.as_str()
        );
    }
    let Some(cell) = selected_cell(screen, panel, row) else {
        return format!(
            "selected panel={} row={} kind=none action=none detail=none",
            panel.as_str(),
            row.saturating_add(1)
        );
    };
    let action = screen_selected_primary_action(screen, state).unwrap_or_else(|| "none".into());
    format!(
        "selected panel={} row={} kind={} title={} action={} detail={}",
        panel.as_str(),
        row.saturating_add(1),
        cell.kind.as_str(),
        terminal_inline(&cell.title),
        terminal_inline(&action),
        terminal_inline(&render_cell_detail_summary(&cell.detail))
    )
}

pub fn screen_selected_actions_line(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> String {
    let panel = state.focused_panel();
    let Some(selection) = selected_cell_index(screen, state) else {
        return format!(
            "screen_selected_actions: panel={} row={} commands=none",
            panel.as_str(),
            state.selection_for(panel).saturating_add(1)
        );
    };
    let Some(cell) = selected_cell(screen, panel, selection) else {
        return format!(
            "screen_selected_actions: panel={} row={} commands=none",
            panel.as_str(),
            selection.saturating_add(1)
        );
    };
    let commands = selected_cell_actions(cell)
        .into_iter()
        .filter(|command| command != "none")
        .collect::<Vec<_>>();
    format!(
        "screen_selected_actions: panel={} row={} commands={}",
        panel.as_str(),
        selection.saturating_add(1),
        if commands.is_empty() {
            "none".into()
        } else {
            terminal_inline(&commands.join(" | "))
        }
    )
}

pub fn screen_selected_actions_json_line(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> String {
    let panel = state.focused_panel();
    let selection =
        selected_cell_index(screen, state).unwrap_or_else(|| state.selection_for(panel));
    let selected = selected_cell(screen, panel, selection);
    let (kind, commands) = selected
        .map(|cell| {
            let commands = selected_cell_actions(cell)
                .into_iter()
                .filter(|command| command != "none")
                .collect::<Vec<_>>();
            (cell.kind.as_str(), commands)
        })
        .unwrap_or(("none", Vec::new()));
    format!(
        "screen_selected_actions_json: {}",
        serde_json::json!({
            "panel": panel.as_str(),
            "row": selection.saturating_add(1),
            "kind": kind,
            "commands": commands,
            "actions": selected_cell_actions_json(selected, &commands),
        })
    )
}

pub fn screen_json_line(screen: &WorkbenchScreen, state: &WorkbenchScreenState) -> String {
    let selected = screen_selected_json_value(screen, state);
    let surface = screen_surface_json_value(screen, state);
    let panels_meta = screen_panels_meta_json_value(screen, state);
    let footer_state = screen_footer_state_json_value(screen, state);
    let state_trace = screen_state_trace_json_value(screen, state, &selected, &surface);
    let modal_kind = surface
        .get("modal")
        .cloned()
        .unwrap_or_else(|| serde_json::json!("none"));
    let overlay_routing = surface
        .get("overlay_routing")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"active_overlay": "composer"}));
    format!(
        "screen_json: {}",
        serde_json::json!({
            "schema": "ikaros-workbench-screen-v1",
            "version": 1,
            "title": terminal_inline(&screen.title),
            "state": {
                "focused_panel": state.focused_panel().as_str(),
                "fullscreen": state.fullscreen(),
                "render_mode": if state.raw_mode() { "raw" } else { "rich" },
                "scroll": {
                    "status": state.scroll_for(WorkbenchScreenPanel::Status),
                    "timeline": state.scroll_for(WorkbenchScreenPanel::Timeline),
                    "main": state.scroll_for(WorkbenchScreenPanel::Main),
                    "side": state.scroll_for(WorkbenchScreenPanel::Side),
                },
                "selection": {
                    "status": state.selection_for(WorkbenchScreenPanel::Status).saturating_add(1),
                    "timeline": state.selection_for(WorkbenchScreenPanel::Timeline).saturating_add(1),
                    "main": state.selection_for(WorkbenchScreenPanel::Main).saturating_add(1),
                    "side": state.selection_for(WorkbenchScreenPanel::Side).saturating_add(1),
                }
            },
            "status": cells_json(&screen.status),
            "panels": {
                "timeline": cells_json(&screen.timeline),
                "main": cells_json(&screen.main),
                "side": cells_json(&screen.side),
            },
            "surface": surface,
            "panels_meta": panels_meta,
            "footer_state": footer_state,
            "state_trace": state_trace,
            "modal_kind": modal_kind,
            "overlay_routing": overlay_routing,
            "modal": screen_modal_json_value(screen),
            "selected": selected,
            "key_bindings": screen_key_bindings_json(),
            "keymap_model": screen_keymap_model_json(screen, state),
            "footer": terminal_inline(&screen.footer),
            "input_hint": terminal_inline(&screen.input_hint),
        })
    )
}
