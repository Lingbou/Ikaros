// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn screen_input_popup_json(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> serde_json::Value {
    if state.command_palette_open {
        return command_palette_overlay_json(screen, state);
    }

    if let Some(search) = find_cell(screen, |cell| cell.title == "history search") {
        let matches = screen
            .main
            .iter()
            .filter(|cell| cell.title.starts_with("history match "))
            .take(6)
            .map(input_popup_cell_json)
            .collect::<Vec<_>>();
        return serde_json::json!({
            "kind": "history_search",
            "summary": input_popup_cell_json(search),
            "query": extract_token_after(&search.detail, "query=")
                .unwrap_or_else(|| "none".into()),
            "matches": extract_token_after(&search.detail, "matches=")
                .unwrap_or_else(|| "0".into()),
            "selected_index": extract_token_after(&search.detail, "selected_index=")
                .unwrap_or_else(|| "0/0".into()),
            "items": matches,
            "actions": {
                "older": "ctrl-r",
                "newer": "ctrl-s",
                "accept": "enter",
                "cancel": "esc",
            },
        });
    }

    let completion = find_cell(screen, |cell| cell.title == "command completion");
    if let Some(completion) = completion {
        let query =
            extract_token_after(&completion.detail, "query=").unwrap_or_else(|| "all".into());
        let selected_command =
            extract_token_after(&completion.detail, "selected=").unwrap_or_default();
        let mut completion_items = screen
            .main
            .iter()
            .filter(|cell| cell.title.starts_with("command /"))
            .take(6)
            .map(|cell| input_popup_command_item_json(screen, cell))
            .collect::<Vec<_>>();
        let selected_index = completion_items
            .iter()
            .position(|item| json_string(item, "command", "") == selected_command)
            .unwrap_or(0);
        for (index, item) in completion_items.iter_mut().enumerate() {
            if let Some(map) = item.as_object_mut() {
                map.insert(
                    "selected".into(),
                    serde_json::Value::Bool(index == selected_index),
                );
            }
        }
        let selected_item = completion_items
            .get(selected_index)
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let selected_command = if completion_items.is_empty() {
            "none".into()
        } else {
            json_string(&selected_item, "command", "none")
        };
        let palette_items = screen
            .main
            .iter()
            .filter(|cell| cell.title.starts_with("palette "))
            .take(6)
            .map(|cell| input_popup_command_item_json(screen, cell))
            .collect::<Vec<_>>();
        let selected_action_label = selected_item
            .get("action_label")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("Run command")
            .to_owned();
        return serde_json::json!({
            "kind": "command_completion",
            "query": query,
            "selected_index": selected_index,
            "selected_count": completion_items.len(),
            "visible_count": completion_items.len(),
            "selected_command": selected_command,
            "selected_action_label": selected_action_label,
            "selected_item": selected_item,
            "empty": completion_items.is_empty(),
            "accept_enabled": !completion_items.is_empty(),
            "enter_noop_when_empty": completion_items.is_empty(),
            "selection_model": "completion_cycle",
            "summary": input_popup_cell_json(completion),
            "context_chips": composer_context_chips_json(screen),
            "completion_items": completion_items,
            "palette_items": palette_items,
            "actions": command_palette_popup_actions_json(false),
        });
    }

    serde_json::Value::Null
}

pub(crate) fn command_palette_overlay_json(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> serde_json::Value {
    let query = state.command_palette_query.as_deref();
    let items = slash_command_palette_items(query, 12);
    let summary = slash_command_palette_summary(query);
    let selected_index = selected_palette_index(state, items.len());
    let selected_command = items
        .get(selected_index)
        .map(command_palette_primary_action)
        .unwrap_or_else(|| "none".into());
    let selected_actions = if selected_command == "none" {
        Vec::new()
    } else {
        vec![selected_command.clone()]
    };
    let selected_position = if items.is_empty() {
        0
    } else {
        selected_index.saturating_add(1)
    };
    let palette_items = items
        .iter()
        .enumerate()
        .map(|(index, item)| command_palette_item_json(screen, item, index == selected_index))
        .collect::<Vec<_>>();
    let selected_item = palette_items
        .get(selected_index)
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let selected_action_label = selected_item
        .get("action_label")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Run command")
        .to_owned();
    serde_json::json!({
        "kind": "command_palette",
        "query": summary.query,
        "query_input": query.unwrap_or_default(),
        "query_editable": true,
        "query_cursor": query.map(str::chars).map(Iterator::count).unwrap_or_default(),
        "selected_index": selected_position,
        "selected_count": palette_items.len(),
        "visible_count": palette_items.len(),
        "match_count": summary.command_count,
        "total_commands": summary.total_commands,
        "list_limit": 12,
        "empty": palette_items.is_empty(),
        "has_query": query.is_some(),
        "accept_enabled": !palette_items.is_empty(),
        "enter_noop_when_empty": palette_items.is_empty(),
        "selection_model": "workbench_palette_state",
        "selected_command": selected_command,
        "selected_action_label": selected_action_label,
        "selected_item": selected_item,
        "summary": {
            "kind": "session",
            "title": "command palette",
            "detail": format!(
                "query={} commands={} total={} effects={} permissions={} surfaces={} selected={} command={}",
                terminal_inline(&summary.query),
                summary.command_count,
                summary.total_commands,
                terminal_inline(&summary.effects),
                terminal_inline(&summary.permissions),
                terminal_inline(&summary.surfaces),
                selected_position,
                terminal_inline(&selected_command),
            ),
            "actions": selected_cell_actions_json(None, &selected_actions),
        },
        "context_chips": composer_context_chips_json(screen),
        "completion_items": [],
        "palette_items": palette_items,
        "actions": command_palette_popup_actions_json(true),
    })
}

pub(crate) fn command_palette_popup_actions_json(open: bool) -> serde_json::Value {
    serde_json::json!({
        "open": open,
        "move_up": "up",
        "move_down": "down",
        "cycle": "tab",
        "accept": "enter",
        "type_to_filter": "text",
        "backspace": "backspace",
        "clear_query": "ctrl-u",
        "cancel": "esc",
        "inspect": "/commands",
        "palette": "/screen --palette",
        "close": "/screen --close-palette",
    })
}

pub(crate) fn command_palette_item_json(
    screen: &WorkbenchScreen,
    item: &SlashCommandPaletteItem,
    selected: bool,
) -> serde_json::Value {
    let command = command_palette_primary_action(item);
    let visible_state = command_visible_state_json(screen, &command);
    serde_json::json!({
        "kind": command_palette_kind_for_effect(item.effect),
        "title": format!("palette {}", item.name),
        "name": item.name,
        "command": command.clone(),
        "usage": terminal_inline(item.usage),
        "argument_model": item.argument_model,
        "effect": item.effect,
        "permissions": terminal_inline(&item.permissions),
        "surfaces": terminal_inline(&item.surfaces),
        "tags": terminal_inline(&item.tags),
        "summary": terminal_inline(item.summary),
        "description": terminal_inline(item.summary),
        "command_class": command_context_class(&command),
        "action": command_action(&command),
        "action_label": command_action_label(&command),
        "category": command_intent(&command),
        "category_tag": item.effect,
        "shortcut": command_shortcut(&command),
        "selected": selected,
        "disabled": false,
        "disabled_reason": null,
        "intent": command_intent(&command),
        "scope": command_scope(&command),
        "risk": command_risk(&command),
        "requires_explicit_action": command_requires_explicit_action(&command),
        "visible_state": visible_state,
        "inspect": format!("/commands {}", item.name),
        "actions": selected_cell_actions_json(None, &[command.clone(), format!("/commands {}", item.name)]),
    })
}

pub(crate) fn command_palette_primary_action(item: &SlashCommandPaletteItem) -> String {
    match item.argument_model {
        "none" | "optional" => item.name.to_owned(),
        _ => format!("/commands {}", item.name),
    }
}

pub(crate) fn command_palette_kind_for_effect(effect: &str) -> &'static str {
    match effect {
        "approval-decision" => "approval",
        "context-inspection" => "context",
        "workspace-inspection" | "workspace-mutation" => "coding",
        "provider-probe" => "model",
        "queue-mutation" | "interrupt" => "continuation",
        "config-mutation" | "agent-mutation" | "session-mutation" => "session",
        _ => "session",
    }
}

pub(crate) fn selected_palette_index(state: &WorkbenchScreenState, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        state.command_palette_selection.min(len.saturating_sub(1))
    }
}

pub(crate) fn selected_palette_command(state: &WorkbenchScreenState) -> Option<String> {
    if !state.command_palette_open {
        return None;
    }
    let query = state.command_palette_query.as_deref();
    let items = slash_command_palette_items(query, 12);
    let selected = items.get(selected_palette_index(state, items.len()))?;
    Some(command_palette_primary_action(selected))
}
