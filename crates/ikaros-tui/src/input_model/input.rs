// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(crate) fn screen_input_model_json(
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
    let context_chips = composer_context_chips_json(screen);
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
        "context_chips": context_chips,
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

pub(crate) fn composer_context_chips_json(screen: &WorkbenchScreen) -> serde_json::Value {
    serde_json::json!([
        session_context_chip_json(screen),
        memory_context_chip_json(screen),
        context_context_chip_json(screen),
    ])
}

pub(crate) fn command_visible_state_json(
    screen: &WorkbenchScreen,
    command: &str,
) -> serde_json::Value {
    match command_root(command) {
        "/session" | "/sessions" | "/resume" | "/new" | "/fork" => {
            session_context_chip_json(screen)
        }
        "/memory" => memory_context_chip_json(screen),
        "/context" => context_context_chip_json(screen),
        _ => serde_json::Value::Null,
    }
}

fn session_context_chip_json(screen: &WorkbenchScreen) -> serde_json::Value {
    let cell = find_cell(screen, |cell| cell.title == "session current")
        .or_else(|| find_cell(screen, |cell| cell.title == "session"));
    let detail = cell.map(|cell| cell.detail.as_str()).unwrap_or_default();
    let session_id = extract_token_after(detail, "id=").unwrap_or_else(|| "unknown".into());
    let agent = extract_token_after(detail, "agent=").unwrap_or_else(|| "unknown".into());
    let attachments = extract_token_after(detail, "attachments=").unwrap_or_else(|| "0".into());
    let continuations = extract_token_after(detail, "continuations=").unwrap_or_else(|| "0".into());
    context_chip_json(
        "session",
        "Session",
        cell.map(|cell| cell.kind.as_str()).unwrap_or("session"),
        "inspect_session",
        "/session",
        "session",
        format!(
            "id={} agent={} attachments={} continuations={}",
            terminal_inline(&session_id),
            terminal_inline(&agent),
            terminal_inline(&attachments),
            terminal_inline(&continuations),
        ),
        serde_json::json!({
            "id": session_id,
            "agent": agent,
            "attachments": attachments,
            "continuations": continuations,
        }),
        cell.is_some(),
    )
}

fn memory_context_chip_json(screen: &WorkbenchScreen) -> serde_json::Value {
    let cell = find_cell(screen, |cell| cell.title == "memory");
    let detail = cell.map(|cell| cell.detail.as_str()).unwrap_or_default();
    let backend = extract_token_after(detail, "backend=").unwrap_or_else(|| "unknown".into());
    let enabled =
        extract_token_after(detail, "context_enabled=").unwrap_or_else(|| "unknown".into());
    let included =
        extract_token_after(detail, "projection_included=").unwrap_or_else(|| "0".into());
    let pending = extract_token_after(detail, "pending_candidates=").unwrap_or_else(|| "0".into());
    let working = extract_token_after(detail, "working_active=").unwrap_or_else(|| "0".into());
    context_chip_json(
        "memory",
        "Memory",
        cell.map(|cell| cell.kind.as_str()).unwrap_or("memory"),
        "inspect_memory_context",
        "/memory",
        "context",
        format!(
            "backend={} enabled={} included={} pending={} working={}",
            terminal_inline(&backend),
            terminal_inline(&enabled),
            terminal_inline(&included),
            terminal_inline(&pending),
            terminal_inline(&working),
        ),
        serde_json::json!({
            "backend": backend,
            "context_enabled": enabled,
            "projection_included": included,
            "pending_candidates": pending,
            "working_active": working,
        }),
        cell.is_some(),
    )
}

fn context_context_chip_json(screen: &WorkbenchScreen) -> serde_json::Value {
    let cell = find_cell(screen, |cell| cell.title == "context overview")
        .or_else(|| find_cell(screen, |cell| cell.title == "context current"));
    let detail = cell.map(|cell| cell.detail.as_str()).unwrap_or_default();
    let disabled = extract_token_after(detail, "disabled=").unwrap_or_else(|| "unknown".into());
    let token_budget =
        extract_token_after(detail, "token_budget=").unwrap_or_else(|| "unknown".into());
    let sections = extract_token_after(detail, "sections=").unwrap_or_else(|| "unknown".into());
    let references = extract_token_after(detail, "references=").unwrap_or_else(|| "unknown".into());
    let memory_limit =
        extract_token_after(detail, "memory_limit=").unwrap_or_else(|| "unknown".into());
    context_chip_json(
        "context",
        "Context",
        cell.map(|cell| cell.kind.as_str()).unwrap_or("context"),
        "inspect_context_budget",
        "/context",
        "context",
        format!(
            "disabled={} token_budget={} sections={} references={} memory_limit={}",
            terminal_inline(&disabled),
            terminal_inline(&token_budget),
            terminal_inline(&sections),
            terminal_inline(&references),
            terminal_inline(&memory_limit),
        ),
        serde_json::json!({
            "disabled": disabled,
            "token_budget": token_budget,
            "sections": sections,
            "references": references,
            "memory_limit": memory_limit,
        }),
        cell.is_some(),
    )
}

fn context_chip_json(
    id: &'static str,
    label: &'static str,
    kind: &str,
    action: &'static str,
    command: &'static str,
    class: &'static str,
    summary: String,
    fields: serde_json::Value,
    visible: bool,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "label": label,
        "kind": kind,
        "class": class,
        "visible": visible,
        "action": action,
        "action_label": command_action_label(command),
        "command": command,
        "summary": terminal_inline(&summary),
        "fields": fields,
    })
}

pub(crate) fn input_popup_cell_json(cell: &WorkbenchCell) -> serde_json::Value {
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

pub(crate) fn input_popup_command_item_json(
    screen: &WorkbenchScreen,
    cell: &WorkbenchCell,
) -> serde_json::Value {
    let commands = selected_cell_actions(cell)
        .into_iter()
        .filter(|command| command != "none")
        .collect::<Vec<_>>();
    let command = command_with_prefix(&commands, "/").unwrap_or_else(|| {
        extract_assignment_display(&cell.detail, "command=", default_cell_command(cell.kind))
    });
    let visible_state = command_visible_state_json(screen, &command);
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
        "command_class": command_context_class(&command),
        "action": command_action(&command),
        "action_label": command_action_label(&command),
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
        "visible_state": visible_state,
        "inspect": command_with_prefix(&commands, "/commands "),
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}

pub(crate) fn command_context_class(command: &str) -> &'static str {
    match command_root(command) {
        "/session" | "/sessions" | "/context" | "/memory" => "inspect_context",
        "/timeline" | "/replay" | "/trace" | "/debug" | "/status" | "/model" | "/provider"
        | "/rag" | "/tools" | "/mcp" | "/api" | "/gateway" | "/diff" => "inspect",
        _ => "command",
    }
}
