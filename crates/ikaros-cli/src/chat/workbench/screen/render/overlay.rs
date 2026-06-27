// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn json_string(
    value: &serde_json::Value,
    key: &str,
    default: &str,
) -> String {
    let Some(value) = value.get(key) else {
        return default.into();
    };
    match value {
        serde_json::Value::String(value) => terminal_inline(value),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null => default.into(),
        value => terminal_inline(&value.to_string()),
    }
}

pub(in crate::chat::workbench::screen) fn screen_overlay_routing_json(
    active_view: &str,
    approval_overlay: &serde_json::Value,
    input_popup: &serde_json::Value,
) -> serde_json::Value {
    let has_approval = !approval_overlay.is_null();
    let popup_kind = input_popup
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let selected_popup_command = command_palette_selected_command(input_popup);
    let active_overlay = if has_approval {
        "approval"
    } else if popup_kind != "none" {
        popup_kind
    } else if matches!(active_view, "input_queue" | "continuation") {
        "queue"
    } else if active_view == "attachments" {
        "attachments"
    } else {
        "composer"
    };
    let modal_scope = if has_approval {
        "approval"
    } else if matches!(popup_kind, "command_palette" | "history_search") {
        popup_kind
    } else {
        "none"
    };
    let enter_target = if has_approval {
        approval_overlay
            .get("routing")
            .and_then(|value| value.get("enter"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("/approval")
            .into()
    } else if popup_kind == "command_palette" {
        selected_popup_command
            .clone()
            .unwrap_or_else(|| "none".into())
    } else if popup_kind == "history_search" {
        "accept_history_match".into()
    } else if popup_kind == "command_completion" {
        "accept_popup_selection".into()
    } else if active_view == "input_queue" {
        "/queue run".into()
    } else if active_view == "continuation" {
        "/debug continuations".into()
    } else if active_view == "attachments" {
        "/attach list".into()
    } else {
        "submit_input".into()
    };
    let alt_enter_target = if has_approval {
        approval_overlay
            .get("routing")
            .and_then(|value| value.get("alt_enter"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("/screen approve-selected")
    } else if popup_kind == "command_palette" {
        "/screen confirm-selected"
    } else {
        "insert_newline_or_confirm_selected"
    };
    let esc_target = if has_approval {
        approval_overlay
            .get("routing")
            .and_then(|value| value.get("esc"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("dismiss_approval_overlay")
    } else if popup_kind == "command_palette" {
        "/screen --close-palette"
    } else if popup_kind != "none" {
        "dismiss_active_surface"
    } else {
        "cancel_input"
    };
    let tab_target = if popup_kind == "command_palette" {
        "/screen --palette --down"
    } else if popup_kind == "command_completion" {
        "cycle_command_completion"
    } else if has_approval {
        "approval_next_action"
    } else {
        "/screen --focus-next"
    };
    let captures_text_input = active_overlay == "command_palette"
        || (!has_approval
            && popup_kind != "history_search"
            && matches!(active_view, "composer")
            && popup_kind != "command_completion");
    let text_target = if active_overlay == "command_palette" {
        "command_palette_filter"
    } else if popup_kind == "history_search" {
        "history_search_query"
    } else if captures_text_input {
        "composer_buffer"
    } else {
        "none"
    };

    serde_json::json!({
        "schema": "ikaros-workbench-overlay-routing-v1",
        "active_view": active_view,
        "active_overlay": active_overlay,
        "active_scope": active_overlay,
        "modal_scope": modal_scope,
        "modal_visible": modal_scope != "none",
        "popup_kind": popup_kind,
        "approval_visible": has_approval,
        "captures_text_input": captures_text_input,
        "text_target": text_target,
        "enter_target": enter_target,
        "alt_enter_target": alt_enter_target,
        "esc_target": esc_target,
        "ctrl_c_target": if has_approval || popup_kind != "none" {
            esc_target
        } else {
            "/cancel all"
        },
        "tab_target": tab_target,
        "selected_popup_command": selected_popup_command.unwrap_or_else(|| "none".into()),
        "priority": [
            "approval",
            "command_palette",
            "history_search",
            "command_completion",
            "queue",
            "composer",
            "panel_navigation",
            "global",
        ],
    })
}

pub(in crate::chat::workbench::screen) fn draw_command_palette_overlay(
    frame: &mut Frame<'_>,
    state: &WorkbenchScreenState,
) {
    if !state.command_palette_open {
        return;
    }
    let area = centered_rect(72, 54, frame.area());
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(command_palette_overlay_text(state))
            .block(
                Block::default()
                    .title("Command Palette")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub(in crate::chat::workbench::screen) fn command_palette_summary_line(
    state: &WorkbenchScreenState,
) -> String {
    let popup = command_palette_overlay_json(state);
    format!(
        "palette query={} selected={}/{} command={} up/down=move enter=open esc=close inspect=/commands",
        json_string(&popup, "query", "all"),
        json_string(&popup, "selected_index", "0"),
        json_string(&popup, "selected_count", "0"),
        json_string(&popup, "selected_command", "none"),
    )
}

pub(in crate::chat::workbench::screen) fn command_palette_overlay_text(
    state: &WorkbenchScreenState,
) -> String {
    let popup = command_palette_overlay_json(state);
    let query = json_string(&popup, "query", "");
    let mut lines = if query.trim().is_empty() || query == "all" {
        vec!["Type to filter commands.".into()]
    } else {
        vec![format!("Filter: {query}")]
    };
    let items = popup
        .get("palette_items")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if items.is_empty() {
        lines.push("No matching commands.".into());
        return lines.join("\n");
    }
    lines.push(String::new());
    for item in items.into_iter().take(10) {
        let marker = if item
            .get("selected")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        {
            ">"
        } else {
            " "
        };
        lines.push(format!(
            "{} {:<18} {}",
            marker,
            json_string(&item, "command", "none"),
            json_string(&item, "summary", "none"),
        ));
    }
    lines.push(String::new());
    lines.push("Enter opens. Esc closes.".into());
    lines.join("\n")
}

pub(in crate::chat::workbench::screen) fn draw_approval_modal_overlay(
    frame: &mut Frame<'_>,
    screen: &WorkbenchScreen,
) {
    let Some(modal) = screen_modal_cell(screen) else {
        return;
    };
    let area = centered_rect(68, 34, frame.area());
    let actions = selected_cell_actions(modal)
        .into_iter()
        .filter(|command| command != "none")
        .collect::<Vec<_>>();
    let approve = command_with_prefix(&actions, "/approval approve ")
        .unwrap_or_else(|| "/screen approve-selected".into());
    let deny = command_with_prefix(&actions, "/approval deny ")
        .unwrap_or_else(|| "/screen deny-selected".into());
    let inspect = approve
        .strip_prefix("/approval approve ")
        .map(|id| format!("/screen --focus side --select-title pending {id}"))
        .unwrap_or_else(|| "/approval list".into());
    let decision = screen_approval_decision_model_json(screen);
    let risk = decision
        .get("risk_breakdown")
        .map(approval_risk_breakdown_text)
        .unwrap_or_else(|| "risk=unknown".into());
    let primary_id = decision
        .get("primary_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let text = format!(
        "> [approval] {}\n{}\n\nRequest:\n  id={}\n  {}\n  guardrails=audit_required,replay_bound,redacted_preview\n\nOptions:\n  enter inspect -> {}\n  alt+a approve -> {}\n  alt+d deny -> {}\n  continue_after_approval=/queue run\n\nActions: {}",
        terminal_inline(&modal.title),
        terminal_inline(&modal.detail),
        terminal_inline(primary_id),
        terminal_inline(&risk),
        terminal_inline(&inspect),
        terminal_inline(&approve),
        terminal_inline(&deny),
        terminal_inline(&actions.join(" | ")),
    );
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(text)
            .block(
                Block::default()
                    .title("Approval Required")
                    .borders(Borders::ALL),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

pub(in crate::chat::workbench::screen) fn approval_risk_breakdown_text(
    value: &serde_json::Value,
) -> String {
    format!(
        "risk high={} provider={} write={} shell={} network={} plugin={} self_modify={}",
        json_string(value, "high_risk", "0"),
        json_string(value, "provider_calls", "0"),
        json_string(value, "workspace_writes", "0"),
        json_string(value, "shell_calls", "0"),
        json_string(value, "network_calls", "0"),
        json_string(value, "plugin_calls", "0"),
        json_string(value, "self_modify_calls", "0"),
    )
}

pub(in crate::chat::workbench::screen) fn centered_rect(
    width_percent: u16,
    height_percent: u16,
    area: Rect,
) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - height_percent) / 2),
            Constraint::Percentage(height_percent),
            Constraint::Percentage((100 - height_percent) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - width_percent) / 2),
            Constraint::Percentage(width_percent),
            Constraint::Percentage((100 - width_percent) / 2),
        ])
        .split(vertical[1])[1]
}
