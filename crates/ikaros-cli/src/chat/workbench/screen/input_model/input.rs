// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn screen_input_model_json(
    screen: &WorkbenchScreen,
    input_popup: &serde_json::Value,
) -> serde_json::Value {
    let composer = find_cell(screen, |cell| cell.title == "composer state");
    let detail = composer
        .map(|cell| cell.detail.as_str())
        .unwrap_or(screen.input_hint.as_str());
    let bottom = find_cell(screen, |cell| cell.title == "bottom pane");
    let active_view = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "active_view="))
        .unwrap_or_else(|| "composer".into());
    let input_popup_kind = input_popup
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let popup_captures_text = matches!(input_popup_kind, "command_palette" | "history_search");
    let mode = extract_token_after(detail, "mode=").unwrap_or_else(|| {
        if input_popup_kind == "history_search" {
            "history_search".into()
        } else if input_popup_kind == "command_palette" {
            "command_palette".into()
        } else {
            "message".into()
        }
    });
    let empty = extract_token_after(detail, "empty=").is_some_and(|value| value == "true");
    let dirty = extract_token_after(detail, "dirty=").is_some_and(|value| value == "true");
    let completion_active =
        extract_token_after(detail, "completion_active=").is_some_and(|value| value == "true");
    let history_search_active =
        extract_token_after(detail, "history_search_active=").is_some_and(|value| value == "true");
    let buffer =
        extract_assignment_span(detail, "buffer=", &[" cursor_view="]).unwrap_or_else(|| {
            extract_assignment_span(&screen.input_hint, "buffer=", &[" view="])
                .unwrap_or_else(|| "none".into())
        });
    let cursor_view = extract_assignment_span(detail, "cursor_view=", &[" submit="])
        .or_else(|| extract_assignment_span(&screen.input_hint, "view=", &[" undo="]))
        .unwrap_or_else(|| terminal_inline(&screen.input_hint));
    serde_json::json!({
        "schema": "ikaros-workbench-composer-v1",
        "source": if composer.is_some() { "composer_state_cell" } else { "input_hint" },
        "retained": true,
        "active_view": active_view.clone(),
        "visible": active_view == "composer",
        "accepts_text": active_view == "composer" && !popup_captures_text,
        "captures_text_input": active_view == "composer" && !popup_captures_text,
        "text_target": if input_popup_kind == "command_palette" {
            "command_palette_filter"
        } else if input_popup_kind == "history_search" {
            "history_search_query"
        } else if active_view == "composer" {
            "composer_buffer"
        } else {
            "none"
        },
        "blocks_input": active_view != "composer" || popup_captures_text,
        "mode": mode,
        "action": extract_token_after(detail, "action=")
            .unwrap_or_else(|| "unknown".into()),
        "buffer": buffer,
        "cursor_view": cursor_view,
        "cursor": extract_token_after(detail, "cursor=")
            .unwrap_or_else(|| "0".into()),
        "chars": extract_token_after(detail, "chars=")
            .unwrap_or_else(|| "0".into()),
        "lines": extract_token_after(detail, "lines=")
            .unwrap_or_else(|| "1".into()),
        "empty": empty,
        "dirty": dirty,
        "history": {
            "entries": extract_token_after(detail, "history_entries=")
                .unwrap_or_else(|| "0".into()),
            "cursor": extract_token_after(detail, "history_cursor=")
                .unwrap_or_else(|| "none".into()),
            "search_active": history_search_active,
            "search_summary": extract_assignment_span(
                detail,
                "history_search_summary=",
                &[" buffer="],
            )
            .unwrap_or_else(|| "none".into()),
            "open": "ctrl-r",
            "older": "ctrl-r",
            "newer": "ctrl-s",
            "accept": "enter",
            "cancel": "esc",
        },
        "completion": {
            "active": completion_active,
            "query": extract_token_after(detail, "completion_query=")
                .unwrap_or_else(|| "none".into()),
            "selected": extract_token_after(detail, "completion_selected=")
                .unwrap_or_else(|| "none".into()),
            "candidates": extract_token_after(detail, "completion_candidates=")
                .unwrap_or_else(|| "none".into()),
            "cycle": "tab",
            "accept": "enter",
            "inspect": "/commands",
            "palette": "/screen --palette",
        },
        "edit": {
            "undo_depth": extract_token_after(detail, "undo=")
                .unwrap_or_else(|| "0".into()),
            "redo_depth": extract_token_after(detail, "redo=")
                .unwrap_or_else(|| "0".into()),
            "undo": extract_token_after(detail, "undo_key=")
                .unwrap_or_else(|| "ctrl-z".into()),
            "redo": extract_token_after(detail, "redo_key=")
                .unwrap_or_else(|| "ctrl-y".into()),
            "word_left": extract_token_after(detail, "word_left=")
                .unwrap_or_else(|| "alt-b".into()),
            "word_right": extract_token_after(detail, "word_right=")
                .unwrap_or_else(|| "alt-f".into()),
            "delete_word": extract_token_after(detail, "delete_word=")
                .unwrap_or_else(|| "ctrl-w/alt-d".into()),
        },
        "submit": {
            "enabled": dirty && active_view == "composer",
            "submit": extract_token_after(detail, "submit=")
                .unwrap_or_else(|| "enter".into()),
            "newline": extract_token_after(detail, "newline=")
                .unwrap_or_else(|| "alt-enter".into()),
            "cancel": extract_token_after(detail, "cancel=")
                .unwrap_or_else(|| "esc".into()),
            "interrupt": extract_token_after(detail, "interrupt=")
                .unwrap_or_else(|| "ctrl-c:exit_or_clear_or_interrupt".into()),
        },
    })
}

pub(in crate::chat::workbench::screen) fn input_popup_cell_json(
    cell: &WorkbenchCell,
) -> serde_json::Value {
    let commands = selected_cell_actions(cell)
        .into_iter()
        .filter(|command| command != "none")
        .collect::<Vec<_>>();
    serde_json::json!({
        "kind": cell.kind.as_str(),
        "title": terminal_inline(&cell.title),
        "detail": terminal_inline(&cell.detail),
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}

pub(in crate::chat::workbench::screen) fn input_popup_command_item_json(
    cell: &WorkbenchCell,
) -> serde_json::Value {
    let commands = selected_cell_actions(cell)
        .into_iter()
        .filter(|command| command != "none")
        .collect::<Vec<_>>();
    let command = command_with_prefix(&commands, "/").unwrap_or_else(|| {
        extract_assignment_display(&cell.detail, "command=", default_cell_command(cell.kind))
    });
    serde_json::json!({
        "kind": cell.kind.as_str(),
        "title": terminal_inline(&cell.title),
        "name": command_root(&command),
        "command": command.clone(),
        "usage": extract_assignment_span(&cell.detail, "usage=", &[" args=", " effect=", " command=", " summary="])
            .unwrap_or_else(|| "unknown".into()),
        "argument_model": extract_token_after(&cell.detail, "args=")
            .unwrap_or_else(|| "unknown".into()),
        "effect": extract_token_after(&cell.detail, "effect=")
            .unwrap_or_else(|| command_risk(&command).into()),
        "permissions": extract_token_after(&cell.detail, "permissions=")
            .unwrap_or_else(|| "unknown".into()),
        "surfaces": extract_token_after(&cell.detail, "surfaces=")
            .unwrap_or_else(|| "unknown".into()),
        "tags": extract_token_after(&cell.detail, "tags=")
            .unwrap_or_else(|| "none".into()),
        "summary": extract_assignment_span(&cell.detail, "summary=", &[" command=", " inspect="])
            .unwrap_or_else(|| "none".into()),
        "description": extract_assignment_span(&cell.detail, "summary=", &[" command=", " inspect="])
            .unwrap_or_else(|| "none".into()),
        "category": command_intent(&command),
        "category_tag": extract_token_after(&cell.detail, "effect=")
            .unwrap_or_else(|| command_risk(&command).into()),
        "shortcut": command_shortcut(&command),
        "selected": false,
        "disabled": false,
        "disabled_reason": null,
        "intent": command_intent(&command),
        "scope": command_scope(&command),
        "risk": command_risk(&command),
        "requires_explicit_action": command_requires_explicit_action(&command),
        "inspect": command_with_prefix(&commands, "/commands "),
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}
