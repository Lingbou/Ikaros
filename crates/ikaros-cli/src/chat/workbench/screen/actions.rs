// SPDX-License-Identifier: GPL-3.0-only

use super::*;
use super::{panels::*, selection::*};

pub(super) fn screen_action_menu_model_json(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
    recovery_model: &serde_json::Value,
    approval_overlay: &serde_json::Value,
    input_popup: &serde_json::Value,
    turn_state: &serde_json::Value,
    overlay_routing: &serde_json::Value,
) -> serde_json::Value {
    let selected = screen_selected_json_value(screen, state);
    let selected_commands =
        json_string_array(selected.get("commands").unwrap_or(&serde_json::Value::Null));
    let selected_items = action_menu_command_items_json("selected", selected_commands);
    let recovery_items = action_menu_recovery_items_json(recovery_model);
    let approval_items = action_menu_approval_items_json(approval_overlay);
    let queue_items = action_menu_queue_items_json(screen);
    let timeline_items = action_menu_timeline_tab_items_json(screen);
    let popup_items = action_menu_popup_items_json(input_popup);
    let global_items = action_menu_global_items_json(input_popup);
    let interrupt_items = action_menu_interrupt_items_json(turn_state);
    let queue_model = screen_queue_panel_json(screen);
    let recovery_active = recovery_model
        .get("needs_attention")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let approval_active = !approval_overlay.is_null();
    let queue_active = queue_model
        .get("needs_attention")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        || queue_model
            .get("selection_state")
            .and_then(|value| value.get("has_active_item"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
        || queue_model
            .get("selection_state")
            .and_then(|value| value.get("can_run"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    let popup_active = !input_popup.is_null();
    let timeline_active = !popup_active && state.focused_panel() == WorkbenchScreenPanel::Timeline;
    let global_active = state
        .action_selection
        .as_deref()
        .is_some_and(action_menu_global_selector);
    let interrupt_active = state
        .action_selection
        .as_deref()
        .is_some_and(action_menu_interrupt_selector);
    let default_group = if approval_active {
        "approval"
    } else if recovery_active {
        "recovery"
    } else if queue_active {
        "queue"
    } else if popup_active {
        "popup"
    } else if timeline_active {
        "timeline"
    } else if interrupt_active {
        "interrupt"
    } else if global_active {
        "global"
    } else {
        "selected"
    };
    let selected_active = !selected_items.is_empty()
        && !recovery_active
        && !approval_active
        && !queue_active
        && !timeline_active
        && !popup_active
        && !global_active
        && !interrupt_active;
    let groups = vec![
        action_menu_group_json("global", "Global", global_active, global_items),
        action_menu_group_json("interrupt", "Interrupt", interrupt_active, interrupt_items),
        action_menu_group_json("recovery", "Recovery", recovery_active, recovery_items),
        action_menu_group_json("approval", "Approval", approval_active, approval_items),
        action_menu_group_json("selected", "Selected", selected_active, selected_items),
        action_menu_group_json("queue", "Queue", queue_active, queue_items),
        action_menu_group_json("timeline", "Timeline Tabs", timeline_active, timeline_items),
        action_menu_group_json("popup", "Input Popup", popup_active, popup_items),
    ];
    let selected_menu_item = state
        .action_selection
        .as_deref()
        .and_then(|selector| action_menu_find_item_by_selector(&groups, selector))
        .unwrap_or(serde_json::Value::Null);
    let implicit_palette_selection = approval_active
        && state
            .action_selection
            .as_deref()
            .is_some_and(action_selection_is_command_palette);
    let selected_menu_item_primary = if implicit_palette_selection {
        None
    } else {
        selected_action_menu_item(&selected_menu_item)
    };
    let primary = selected_menu_item_primary
        .or_else(|| active_action_menu_primary(&groups))
        .or_else(|| first_action_menu_item(&groups))
        .unwrap_or_else(|| action_menu_item_json("none", "No action", "none", None, false));

    serde_json::json!({
        "schema": "ikaros-workbench-action-menu-v1",
        "focused_panel": state.focused_panel().as_str(),
        "selection_selector": state
            .action_selection
            .clone()
            .unwrap_or_else(|| "none".into()),
        "selected": selected,
        "selected_menu_item": selected_menu_item,
        "primary": primary,
        "priority": {
            "default_group": default_group,
            "active_overlay": overlay_routing
                .get("active_overlay")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("composer")),
            "modal_scope": overlay_routing
                .get("modal_scope")
                .cloned()
                .unwrap_or_else(|| serde_json::json!("none")),
            "explicit_selection_overrides_default": !selected_menu_item.is_null()
                && !implicit_palette_selection,
            "approval_before_popup": true,
            "queue_before_timeline": true,
        },
        "routing": overlay_routing.clone(),
        "groups": groups,
        "bindings": {
            "open": "enter",
            "confirm": "alt-enter",
            "approve": "alt+a",
            "deny": "alt+d",
            "cancel": "alt+c",
            "clear": "alt+x",
            "palette": "/screen --palette",
            "select_help": "/screen --select-action global_help",
            "select_palette": "/screen --select-action global_palette",
            "select_interrupt": "/screen --select-action interrupt_cancel",
            "select_primary": "/screen --select-action primary",
            "select_recovery": "/screen --select-action recovery_primary",
            "select_approval": "/screen --select-action approval_approve",
            "select_queue": "/screen --select-action queue_cancel_all",
            "select_timeline": "/screen --select-action timeline_all",
        },
    })
}

pub(super) fn selected_action_menu_item(item: &serde_json::Value) -> Option<serde_json::Value> {
    item.as_object()?;
    item.get("command")
        .and_then(serde_json::Value::as_str)
        .filter(|command| *command != "none")
        .map(|_| item.clone())
}

pub(super) fn active_action_menu_primary(
    groups: &[serde_json::Value],
) -> Option<serde_json::Value> {
    const PRIORITY: &[&str] = &[
        "approval",
        "recovery",
        "queue",
        "popup",
        "timeline",
        "selected",
        "interrupt",
        "global",
    ];
    PRIORITY.iter().find_map(|id| {
        groups
            .iter()
            .find(|group| group.get("id").and_then(serde_json::Value::as_str) == Some(*id))
            .and_then(|group| {
                if !group
                    .get("active")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
                {
                    return None;
                }
                group
                    .get("items")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|items| items.first())
                    .cloned()
            })
    })
}

pub(super) fn first_action_menu_item(groups: &[serde_json::Value]) -> Option<serde_json::Value> {
    first_action_menu_item_in_groups(groups, |group_id| {
        !matches!(group_id, Some("global") | Some("interrupt"))
    })
    .or_else(|| first_action_menu_item_in_groups(groups, |_| true))
}

pub(super) fn first_action_menu_item_in_groups(
    groups: &[serde_json::Value],
    mut include_group: impl FnMut(Option<&str>) -> bool,
) -> Option<serde_json::Value> {
    groups
        .iter()
        .filter(|group| include_group(group.get("id").and_then(serde_json::Value::as_str)))
        .filter_map(|group| group.get("items").and_then(serde_json::Value::as_array))
        .find_map(|items| items.first().cloned())
}

pub(super) fn action_menu_find_item_by_selector(
    groups: &[serde_json::Value],
    selector: &str,
) -> Option<serde_json::Value> {
    let selector = selector.to_ascii_lowercase();
    groups.iter().find_map(|group| {
        let group_id = group.get("id").and_then(serde_json::Value::as_str);
        group
            .get("items")
            .and_then(serde_json::Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| action_menu_item_matches_selector(group_id, item, &selector))
            })
            .cloned()
    })
}

pub(super) fn action_menu_group_json(
    id: &str,
    label: &str,
    active: bool,
    items: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "label": label,
        "active": active,
        "empty": items.is_empty(),
        "items": items,
    })
}

pub(super) fn action_menu_global_items_json(
    input_popup: &serde_json::Value,
) -> Vec<serde_json::Value> {
    let palette_command =
        command_palette_selected_command(input_popup).unwrap_or_else(|| "/screen --palette".into());
    let palette_label = if palette_command == "/screen --palette" {
        "Command palette".into()
    } else {
        format!("Open {}", command_action_label(&palette_command))
    };
    let palette_shortcut = if palette_command == "/screen --palette" {
        Some("f5")
    } else {
        Some("enter")
    };
    vec![
        action_menu_item_json("global_help", "Help", "/help", Some("f1"), false),
        action_menu_item_json(
            "global_palette",
            &palette_label,
            &palette_command,
            palette_shortcut,
            command_requires_explicit_action(&palette_command),
        ),
    ]
}

pub(super) fn action_menu_global_selector(selector: &str) -> bool {
    matches!(
        compact_action_selector(selector).as_str(),
        "help" | "globalhelp" | "f1" | "palette" | "commandpalette" | "globalpalette" | "f5"
    )
}

pub(super) fn action_selection_is_command_palette(selector: &str) -> bool {
    matches!(
        compact_action_selector(selector).as_str(),
        "palette" | "commandpalette" | "globalpalette" | "f5"
    )
}

pub(super) fn action_menu_interrupt_selector(selector: &str) -> bool {
    matches!(
        compact_action_selector(selector).as_str(),
        "interrupt"
            | "interruptcancel"
            | "cancelactivework"
            | "alti"
            | "ctrli"
            | "ctrlcalti"
            | "traceactivework"
            | "interrupttrace"
            | "timeline"
            | "interrupttimeline"
    )
}

pub(super) fn action_menu_interrupt_items_json(
    turn_state: &serde_json::Value,
) -> Vec<serde_json::Value> {
    let can_cancel = turn_state
        .get("can_cancel")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !can_cancel {
        return Vec::new();
    }
    let state = turn_state
        .get("state")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("active");
    let reason = turn_state
        .get("blocking_reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("active_work");
    vec![
        action_menu_item_json(
            "interrupt_cancel",
            &format!("Cancel active work ({state})"),
            "/cancel all",
            Some("ctrl-c/alt-i"),
            false,
        ),
        action_menu_item_json(
            "interrupt_trace",
            &format!("Trace active work ({reason})"),
            "/trace",
            None,
            false,
        ),
        action_menu_item_json(
            "interrupt_timeline",
            "Open timeline",
            "/timeline",
            Some("ctrl-t"),
            false,
        ),
    ]
}

pub(super) fn action_menu_command_items_json(
    source: &str,
    commands: Vec<String>,
) -> Vec<serde_json::Value> {
    commands
        .into_iter()
        .enumerate()
        .map(|(index, command)| {
            action_menu_item_json(
                &format!("{source}_{index}"),
                command_action_label(&command),
                &command,
                command_shortcut(&command),
                command_requires_explicit_action(&command),
            )
        })
        .collect()
}

pub(super) fn action_menu_recovery_items_json(
    recovery_model: &serde_json::Value,
) -> Vec<serde_json::Value> {
    let mut items = Vec::new();
    if let Some(primary) = recovery_model.get("primary") {
        if let Some(item) = action_menu_item_from_value("recovery_primary", primary) {
            items.push(item);
        }
    }
    if let Some(secondary) = recovery_model
        .get("secondary")
        .and_then(serde_json::Value::as_array)
    {
        items.extend(secondary.iter().enumerate().filter_map(|(index, value)| {
            action_menu_item_from_value(&format!("recovery_secondary_{index}"), value)
        }));
    }
    items
}

pub(super) fn action_menu_approval_items_json(
    approval_overlay: &serde_json::Value,
) -> Vec<serde_json::Value> {
    approval_overlay
        .get("options")
        .and_then(serde_json::Value::as_array)
        .map(|options| {
            options
                .iter()
                .enumerate()
                .filter_map(|(index, value)| {
                    let id = value
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .map(|id| format!("approval_{id}"))
                        .unwrap_or_else(|| format!("approval_{index}"));
                    action_menu_item_from_value(&id, value)
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn action_menu_queue_items_json(screen: &WorkbenchScreen) -> Vec<serde_json::Value> {
    let queue = screen_queue_panel_json(screen);
    let mut items = Vec::new();
    if let Some(primary) = queue.get("primary") {
        if let Some(item) = action_menu_item_from_value("queue_primary", primary) {
            items.push(item);
        }
    }
    let recovery = queue
        .get("recovery")
        .and_then(|value| value.get("actions"))
        .and_then(serde_json::Value::as_object);
    if let Some(actions) = recovery {
        items.extend(actions.iter().filter_map(|(id, command)| {
            command.as_str().map(|command| {
                action_menu_item_json(
                    &format!("queue_{id}"),
                    command_action_label(command),
                    command,
                    command_shortcut(command),
                    command_requires_explicit_action(command),
                )
            })
        }));
    }
    items
}

pub(super) fn action_menu_timeline_tab_items_json(
    screen: &WorkbenchScreen,
) -> Vec<serde_json::Value> {
    screen_timeline_tabs(screen)
        .into_iter()
        .filter(|tab| tab.count > 0 || tab.attention || matches!(tab.id, "all" | "error"))
        .map(|tab| {
            action_menu_item_json(
                &format!("timeline_{}", tab.id),
                &format!("Timeline: {}", tab.label),
                &tab.timeline_command,
                tab.shortcut,
                false,
            )
        })
        .collect()
}

pub(super) fn action_menu_popup_items_json(
    input_popup: &serde_json::Value,
) -> Vec<serde_json::Value> {
    if input_popup.is_null() {
        return Vec::new();
    }
    let kind = input_popup
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let mut items = command_palette_selected_command(input_popup)
        .map(|command| {
            vec![action_menu_item_json(
                "popup_accept",
                &format!("Open {}", command_action_label(&command)),
                &command,
                Some("enter"),
                command_requires_explicit_action(&command),
            )]
        })
        .unwrap_or_else(|| {
            vec![action_menu_item_json(
                "popup_accept",
                "Accept popup selection",
                "accept_popup_selection",
                Some("enter"),
                false,
            )]
        });
    if matches!(kind, "command_completion" | "command_palette") {
        items.push(action_menu_item_json(
            "popup_cycle",
            "Cycle command candidates",
            "cycle_command_completion",
            Some("tab"),
            false,
        ));
        items.push(action_menu_item_json(
            "popup_inspect",
            "Inspect command registry",
            "/commands",
            None,
            false,
        ));
    }
    items.push(action_menu_item_json(
        "popup_cancel",
        "Dismiss popup",
        "dismiss_active_surface",
        Some("esc"),
        false,
    ));
    items
}

pub(super) fn command_palette_selected_command(input_popup: &serde_json::Value) -> Option<String> {
    input_popup
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .filter(|kind| *kind == "command_palette")?;
    input_popup
        .get("selected_command")
        .and_then(serde_json::Value::as_str)
        .filter(|command| *command != "none")
        .map(ToOwned::to_owned)
}

pub(super) fn action_menu_item_from_value(
    id: &str,
    value: &serde_json::Value,
) -> Option<serde_json::Value> {
    let command = value
        .get("command")
        .and_then(serde_json::Value::as_str)
        .filter(|command| *command != "none")?;
    let label = value
        .get("label")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| command_action_label(command));
    let shortcut = value
        .get("shortcut")
        .and_then(serde_json::Value::as_str)
        .or_else(|| value.get("key").and_then(serde_json::Value::as_str))
        .or_else(|| command_shortcut(command));
    let requires_explicit = value
        .get("requires_explicit_action")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or_else(|| command_requires_explicit_action(command));
    Some(action_menu_item_json(
        id,
        label,
        command,
        shortcut,
        requires_explicit,
    ))
}

pub(super) fn action_menu_item_json(
    id: &str,
    label: &str,
    command: &str,
    shortcut: Option<&str>,
    requires_explicit_action: bool,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "label": label,
        "command": command,
        "shortcut": shortcut,
        "intent": command_intent(command),
        "scope": command_scope(command),
        "risk": command_risk(command),
        "requires_explicit_action": requires_explicit_action,
    })
}

pub(super) fn command_action_label(command: &str) -> &str {
    match command {
        "none" => "No action",
        "enter" => "Submit input",
        "esc" => "Cancel input",
        "accept_popup_selection" => "Accept popup selection",
        "cycle_command_completion" => "Cycle command candidates",
        "dismiss_active_surface" => "Dismiss active surface",
        _ if command.starts_with("/help") => "Help",
        _ if command.starts_with("/screen --palette") => "Command palette",
        _ if command.starts_with("/commands --palette") => "Command palette",
        _ if command.starts_with("/screen approve-selected") => "Approve selected",
        _ if command.starts_with("/screen deny-selected") => "Deny selected",
        _ if command.starts_with("/approval approve ") => "Approve request",
        _ if command.starts_with("/approval deny ") => "Deny request",
        _ if command.starts_with("/cancel") => "Cancel work",
        _ if command.starts_with("/queue run") => "Run queue",
        _ if command.starts_with("/queue retry") || command.starts_with("/queue requeue") => {
            "Retry continuation"
        }
        _ if command.starts_with("/trace") => "Trace",
        _ if command.starts_with("/timeline") => "Timeline",
        _ if command.starts_with("/debug") => "Debug",
        _ if command.starts_with("/code review") => "Review code turn",
        _ if command.starts_with("/code test") => "Run code tests",
        _ if command.starts_with("/code rollback") => "Rollback code turn",
        _ if command.starts_with("/provider") => "Provider",
        _ if command.starts_with("/budget") => "Budget",
        _ => "Run command",
    }
}

pub(super) fn json_string_array(value: &serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}
