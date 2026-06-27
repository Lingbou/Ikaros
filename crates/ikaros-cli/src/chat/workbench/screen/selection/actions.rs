// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn side_panel_title(
    screen: &WorkbenchScreen,
) -> &'static str {
    if screen_modal_cell(screen).is_some() {
        "Approvals / Queue Modal"
    } else {
        "Approvals / Queue"
    }
}

pub(in crate::chat::workbench::screen) fn screen_modal_summary(cell: &WorkbenchCell) -> String {
    format!(
        "Modal approval title={} actions={}",
        terminal_inline(&cell.title),
        terminal_inline(&selected_cell_actions(cell).join(" | "))
    )
}

pub(in crate::chat::workbench::screen) fn screen_modal_json_value(
    screen: &WorkbenchScreen,
) -> serde_json::Value {
    let Some(cell) = screen_modal_cell(screen) else {
        return serde_json::Value::Null;
    };
    let commands = selected_cell_actions(cell)
        .into_iter()
        .filter(|command| command != "none")
        .collect::<Vec<_>>();
    let approve = command_with_prefix(&commands, "/approval approve ");
    let deny = command_with_prefix(&commands, "/approval deny ");
    let primary_id = approve
        .as_deref()
        .and_then(approval_id_from_decision_command)
        .or_else(|| deny.as_deref().and_then(approval_id_from_decision_command));
    let inspect = primary_id
        .as_deref()
        .map(|id| format!("/screen --focus side --select-title pending {id}"))
        .unwrap_or_else(|| "/approval list".into());
    let options = approval_modal_options_json(
        approve.as_deref(),
        deny.as_deref(),
        &inspect,
        primary_id.as_deref(),
    );
    let primary = serde_json::json!({
        "id": "approval_inspect",
        "label": "Inspect request",
        "command": inspect,
        "shortcut": "enter",
        "intent": command_intent(&inspect),
        "scope": command_scope(&inspect),
        "risk": command_risk(&inspect),
        "requires_explicit_action": false,
    });
    serde_json::json!({
        "kind": "approval",
        "title": terminal_inline(&cell.title),
        "detail": terminal_inline(&cell.detail),
        "primary_id": primary_id,
        "approve": approve
            .clone()
            .unwrap_or_else(|| "/screen approve-selected".into()),
        "deny": deny
            .clone()
            .unwrap_or_else(|| "/screen deny-selected".into()),
        "inspect": inspect,
        "blocking": true,
        "terminal_title": "Action Required",
        "default_option": "inspect",
        "primary": primary,
        "routing": {
            "enter": inspect,
            "alt_enter": "/screen approve-selected",
            "approve": approve
                .clone()
                .unwrap_or_else(|| "/screen approve-selected".into()),
            "deny": deny
                .clone()
                .unwrap_or_else(|| "/screen deny-selected".into()),
            "esc": "dismiss_approval_overlay",
            "priority": "approval_before_popup",
        },
        "options": options,
        "commands": commands,
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}

pub(in crate::chat::workbench::screen) fn approval_modal_options_json(
    approve: Option<&str>,
    deny: Option<&str>,
    inspect: &str,
    primary_id: Option<&str>,
) -> Vec<serde_json::Value> {
    vec![
        serde_json::json!({
            "id": "inspect",
            "label": "Inspect request",
            "shortcut": "enter",
            "decision": "inspect",
            "risk": "read",
            "approval_id": primary_id,
            "command": inspect,
            "requires_explicit_action": false,
        }),
        serde_json::json!({
            "id": "approve",
            "label": "Approve",
            "shortcut": "alt+a",
            "decision": "approve",
            "risk": "approval-decision",
            "approval_id": primary_id,
            "command": approve.unwrap_or("/screen approve-selected"),
            "requires_explicit_action": true,
        }),
        serde_json::json!({
            "id": "deny",
            "label": "Deny",
            "shortcut": "alt+d",
            "decision": "deny",
            "risk": "approval-decision",
            "approval_id": primary_id,
            "command": deny.unwrap_or("/screen deny-selected"),
            "requires_explicit_action": true,
        }),
    ]
}

pub(in crate::chat::workbench::screen) fn screen_selected_json_value(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> serde_json::Value {
    let panel = state.focused_panel();
    let selection =
        selected_cell_index(screen, state).unwrap_or_else(|| state.selection_for(panel));
    let Some(cell) = selected_cell(screen, panel, selection) else {
        return serde_json::json!({
            "panel": panel.as_str(),
            "row": selection.saturating_add(1),
            "kind": "none",
            "title": "none",
            "detail": "none",
            "commands": [],
            "actions": selected_cell_actions_json(None, &[]),
        });
    };
    let commands = selected_cell_actions(cell)
        .into_iter()
        .filter(|command| command != "none")
        .collect::<Vec<_>>();
    serde_json::json!({
        "panel": panel.as_str(),
        "row": selection.saturating_add(1),
        "kind": cell.kind.as_str(),
        "title": terminal_inline(&cell.title),
        "detail": terminal_inline(&cell.detail),
        "commands": commands,
        "actions": selected_cell_actions_json(Some(cell), &commands),
    })
}

pub(in crate::chat::workbench::screen) fn selected_cell_actions_json(
    cell: Option<&WorkbenchCell>,
    commands: &[String],
) -> serde_json::Value {
    let primary = commands.first().cloned();
    let intents = selected_cell_intents_json(commands);
    let replay = selected_replay_actions_json(commands);
    let approval = cell
        .filter(|cell| matches!(cell.kind, WorkbenchCellKind::Approval))
        .map(|_| {
            serde_json::json!({
                "approve": command_with_prefix(commands, "/approval approve "),
                "deny": command_with_prefix(commands, "/approval deny "),
                "approve_selected": selected_command_action(
                    commands,
                    "/approval approve ",
                    "/screen approve-selected",
                ),
                "deny_selected": selected_command_action(
                    commands,
                    "/approval deny ",
                    "/screen deny-selected",
                ),
            })
        })
        .unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "primary": primary,
        "open_selected": commands.first().map(|_| "/screen open-selected"),
        "confirm_selected": commands
            .iter()
            .any(|command| command_requires_explicit_action(command))
            .then_some("/screen confirm-selected"),
        "approve_selected": selected_command_action(
            commands,
            "/approval approve ",
            "/screen approve-selected",
        ),
        "deny_selected": selected_command_action(
            commands,
            "/approval deny ",
            "/screen deny-selected",
        ),
        "cancel_selected": selected_command_action(
            commands,
            "/cancel ",
            "/screen cancel-selected",
        ),
        "clear_selected": command_with_prefix(commands, "/queue remove ")
            .or_else(|| command_with_prefix(commands, "/attach remove "))
            .map(|_| "/screen clear-selected")
            .or_else(|| selected_command_action(
                commands,
                "/screen clear-selected",
                "/screen clear-selected",
            )),
        "approval": approval,
        "replay": replay,
        "intents": intents,
    })
}

pub(in crate::chat::workbench::screen) fn selected_replay_actions_json(
    commands: &[String],
) -> serde_json::Value {
    serde_json::json!({
        "timeline": command_with_prefix(commands, "/timeline"),
        "replay": command_with_prefix(commands, "/replay"),
        "trace": command_with_prefix(commands, "/trace"),
        "debug": command_with_prefix(commands, "/debug"),
        "failed": command_with_prefixes(commands, &[
            "/timeline --failed",
            "/replay --failed",
            "/trace --failed",
        ]),
        "approval": command_with_prefixes(commands, &[
            "/timeline --approval",
            "/replay --approval",
            "/trace --approval",
        ]),
    })
}

pub(in crate::chat::workbench::screen) fn selected_cell_intents_json(
    commands: &[String],
) -> Vec<serde_json::Value> {
    commands
        .iter()
        .map(|command| {
            let command = terminal_inline(command);
            serde_json::json!({
                "intent": command_intent(&command),
                "scope": command_scope(&command),
                "action": command_action(&command),
                "target": command_target(&command),
                "risk": command_risk(&command),
                "requires_explicit_action": command_requires_explicit_action(&command),
                "command": command,
            })
        })
        .collect()
}

pub(in crate::chat::workbench::screen) fn command_intent(command: &str) -> &'static str {
    match command_root(command) {
        "/screen" | "/help" => "ui",
        "/commands" => "registry",
        "/approval" | "/approvals" => "approval",
        "/cancel" => "interrupt",
        "/queue" => "input_queue",
        "/attach" => "attachment",
        "/timeline" | "/replay" | "/trace" | "/debug" => "replay",
        "/code" | "/diff" | "/review" | "/rollback" => "coding",
        "/provider" | "/model" | "/budget" => "provider",
        "/context" => "context",
        "/memory" => "memory",
        "/rag" => "rag",
        "/tools" | "/mcp" => "tools",
        "/browser" | "/web" | "/vision" | "/image" | "/api" | "/gateway" => "surface",
        _ => "command",
    }
}

pub(in crate::chat::workbench::screen) fn command_scope(command: &str) -> &'static str {
    match command_root(command) {
        "/screen" | "/help" | "/commands" => "workbench",
        "/approval" | "/approvals" => "approval",
        "/queue" | "/attach" | "/cancel" => "turn",
        "/timeline" | "/replay" | "/trace" | "/debug" => "session",
        "/code" | "/diff" | "/review" | "/rollback" => "workspace",
        "/provider" | "/model" | "/budget" => "provider",
        "/context" | "/memory" | "/rag" | "/tools" | "/mcp" => "runtime",
        "/browser" | "/web" | "/vision" | "/image" | "/api" | "/gateway" => "integration",
        _ => "workbench",
    }
}

pub(in crate::chat::workbench::screen) fn command_action(command: &str) -> String {
    let mut parts = command.split_whitespace();
    let root = parts.next().unwrap_or("none");
    let action = parts.next().unwrap_or(match root {
        "/timeline" | "/replay" | "/trace" | "/debug" => "open",
        "/provider" | "/model" | "/context" | "/memory" | "/rag" | "/tools" | "/mcp" => "inspect",
        _ => "run",
    });
    if root == "/screen" {
        return action.replace('-', "_");
    }
    if root == "/approval" && matches!(action, "approve" | "deny") {
        return format!("approval_{action}");
    }
    action.replace('-', "_")
}

pub(in crate::chat::workbench::screen) fn command_target(command: &str) -> Option<String> {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        ["/approval", "approve" | "deny", id, ..] => Some((*id).to_owned()),
        ["/cancel", id, ..] => Some((*id).to_owned()),
        ["/queue", "remove", id, ..] => Some((*id).to_owned()),
        ["/queue", "retry" | "requeue", id, ..] => Some((*id).to_owned()),
        ["/attach", "remove", id, ..] => Some((*id).to_owned()),
        ["/timeline" | "/replay" | "/trace" | "/debug", value, ..] if !value.starts_with('-') => {
            Some((*value).to_owned())
        }
        ["/screen", action, ..] => Some(action.replace('-', "_")),
        _ => None,
    }
}

pub(in crate::chat::workbench::screen) fn command_risk(command: &str) -> &'static str {
    match command_root(command) {
        "/approval" | "/approvals" => "approval-decision",
        "/cancel" => "interrupt",
        "/queue" | "/attach" | "/screen" => "local-ui",
        "/code" | "/rollback" => "workspace-mutation",
        "/browser" | "/web" | "/vision" | "/image" | "/provider" | "/gateway" => {
            "network-or-provider"
        }
        "/budget" | "/agent" | "/session" => "runtime-mutation",
        _ => "read",
    }
}

pub(in crate::chat) fn command_requires_explicit_action(command: &str) -> bool {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        ["/approval" | "/approvals", "approve" | "deny", ..] => true,
        [
            "/screen",
            "approve-selected" | "approve" | "deny-selected" | "deny",
            ..,
        ] => true,
        ["/code" | "/rollback", ..] => true,
        ["/queue", "remove" | "clear" | "retry" | "requeue", ..] => true,
        ["/attach", "remove" | "clear", ..] => true,
        ["/provider", ..] if parts.contains(&"--live") => true,
        ["/browser", command, ..]
            if !matches!(
                *command,
                "status" | "list" | "supervisor" | "supervisor-status"
            ) =>
        {
            true
        }
        ["/web", "search" | "extract", ..] => true,
        ["/vision", "describe", target, ..] if !target.starts_with('-') => true,
        ["/image", "generate", prompt, ..] if !prompt.starts_with('-') => true,
        ["/gateway", "daemon" | "adapter", command, ..]
            if !matches!(*command, "status" | "list") =>
        {
            true
        }
        ["/budget", ..] | ["/agent", ..] | ["/session", ..] => true,
        _ => false,
    }
}

pub(in crate::chat::workbench::screen) fn command_shortcut(command: &str) -> Option<&'static str> {
    match command_root(command) {
        "/help" => Some("f1"),
        "/commands" => Some("f5"),
        "/screen" => Some("tab"),
        "/approval" | "/approvals" => Some("alt-a/alt-d"),
        "/cancel" => Some("alt-c"),
        "/queue" | "/attach" => Some("alt-x"),
        "/timeline" | "/replay" | "/trace" | "/debug" => Some("ctrl-t"),
        _ => None,
    }
}

pub(in crate::chat::workbench::screen) fn command_root(command: &str) -> &str {
    command.split_whitespace().next().unwrap_or("none")
}

pub(in crate::chat::workbench::screen) fn selected_command_action(
    commands: &[String],
    prefix: &str,
    selected_command: &'static str,
) -> Option<&'static str> {
    commands
        .iter()
        .any(|command| command == selected_command || command.starts_with(prefix))
        .then_some(selected_command)
}

pub(in crate::chat::workbench::screen) fn command_with_prefix(
    commands: &[String],
    prefix: &str,
) -> Option<String> {
    commands
        .iter()
        .find(|command| command.starts_with(prefix))
        .cloned()
}

pub(in crate::chat::workbench::screen) fn approval_id_from_decision_command(
    command: &str,
) -> Option<String> {
    command
        .strip_prefix("/approval approve ")
        .or_else(|| command.strip_prefix("/approval deny "))
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(terminal_inline)
}

pub(in crate::chat::workbench::screen) fn command_with_prefixes(
    commands: &[String],
    prefixes: &[&str],
) -> Option<String> {
    prefixes
        .iter()
        .find_map(|prefix| command_with_prefix(commands, prefix))
}

pub(in crate::chat::workbench::screen) fn cells_json(
    cells: &[WorkbenchCell],
) -> Vec<serde_json::Value> {
    cells
        .iter()
        .map(|cell| {
            serde_json::json!({
                "kind": cell.kind.as_str(),
                "title": terminal_inline(&cell.title),
                "detail": terminal_inline(&cell.detail),
            })
        })
        .collect()
}
