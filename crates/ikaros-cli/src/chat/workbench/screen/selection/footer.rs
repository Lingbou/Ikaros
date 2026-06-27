// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn selected_action_footer(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> String {
    screen_selected_primary_action(screen, state)
        .map(|command| {
            let confirm = if command_requires_explicit_action(&command) {
                " confirm=/screen confirm-selected"
            } else {
                ""
            };
            format!("enter={}{}", terminal_inline(&command), confirm)
        })
        .unwrap_or_else(|| "enter=none".into())
}

pub(in crate::chat) fn screen_selected_primary_action(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> Option<String> {
    let has_action_selection = state.action_selection.is_some();
    let implicit_palette_selection = state
        .action_selection
        .as_deref()
        .is_some_and(action_selection_is_command_palette)
        && state.command_palette_open;
    if (!has_action_selection || implicit_palette_selection) && screen_modal_cell(screen).is_some()
    {
        if let Some(command) = screen_approval_primary_command(screen) {
            return Some(command);
        }
    }
    if let Some(command) = selected_palette_command(state) {
        return Some(command);
    }
    let cell = if has_action_selection {
        if let Some(command) = selected_action_command(screen, state) {
            return Some(command);
        }
        let panel = state.focused_panel();
        selected_cell_index(screen, state)
            .and_then(|selection| selected_cell(screen, panel, selection))?
    } else {
        let panel = state.focused_panel();
        match selected_cell_index(screen, state)
            .and_then(|selection| selected_cell(screen, panel, selection))
        {
            Some(cell) => cell,
            None => return screen_recovery_primary_command(screen),
        }
    };
    selected_cell_actions(cell)
        .into_iter()
        .find(|command| command != "none")
}

pub(in crate::chat::workbench::screen) fn screen_recovery_primary_command(
    screen: &WorkbenchScreen,
) -> Option<String> {
    let progress = screen_surface_progress_json(screen);
    let input_model = screen_input_model_json(screen, &serde_json::Value::Null);
    let turn_state = screen_turn_state_model_json(screen, &progress, &input_model);
    let state = json_string(&turn_state, "state", "idle");
    let recovery = screen_recovery_model_json(screen, &turn_state);
    let recovery_status = json_string(&recovery, "status", "idle");
    if !matches!(recovery_status.as_str(), "blocked" | "recoverable")
        && !(recovery_status == "ready" && state == "queued")
    {
        return None;
    }
    recovery
        .get("primary")
        .and_then(|value| value.get("command"))
        .and_then(serde_json::Value::as_str)
        .filter(|command| command.starts_with('/') && *command != "none")
        .map(ToOwned::to_owned)
}

pub(in crate::chat::workbench::screen) fn screen_approval_primary_command(
    screen: &WorkbenchScreen,
) -> Option<String> {
    let modal = screen_modal_json_value(screen);
    modal
        .get("primary")
        .and_then(|item| item.get("command"))
        .and_then(serde_json::Value::as_str)
        .filter(|command| *command != "none")
        .map(ToOwned::to_owned)
}

pub(in crate::chat::workbench::screen) fn selected_cell_actions(
    cell: &WorkbenchCell,
) -> Vec<String> {
    let mut commands = Vec::new();
    let combined = format!("{} {}", cell.title, cell.detail);
    if let Some(turn_id) = extract_token_after(&combined, "turn=") {
        commands.push(format!("/timeline {turn_id}"));
        commands.push(format!("/trace {turn_id}"));
        commands.push(format!("/debug {turn_id}"));
    }
    commands.extend(extract_assignment_commands(&combined, "approve="));
    commands.extend(extract_assignment_commands(&combined, "approval="));
    commands.extend(extract_assignment_commands(&combined, "deny="));
    commands.extend(extract_assignment_commands(&combined, "continue="));
    commands.extend(extract_assignment_commands(&combined, "cancel="));
    commands.extend(extract_assignment_commands(&combined, "retry="));
    commands.extend(extract_assignment_commands(&combined, "requeue="));
    commands.extend(extract_assignment_commands(&combined, "run="));
    commands.extend(extract_assignment_commands(&combined, "clear="));
    commands.extend(extract_assignment_commands(&combined, "open="));
    commands.extend(extract_assignment_commands(&combined, "command="));
    commands.extend(extract_assignment_commands(&combined, "plan="));
    commands.extend(extract_assignment_commands(&combined, "diff="));
    commands.extend(extract_assignment_commands(&combined, "apply="));
    commands.extend(extract_assignment_commands(&combined, "test="));
    commands.extend(extract_assignment_commands(&combined, "review="));
    commands.extend(extract_assignment_commands(&combined, "rollback="));
    commands.extend(extract_assignment_commands(&combined, "workflow="));
    commands.extend(extract_assignment_commands(&combined, "budget="));
    commands.extend(extract_assignment_commands(&combined, "raise="));
    commands.extend(extract_assignment_commands(&combined, "disable="));
    commands.extend(extract_assignment_commands(&combined, "timeline="));
    commands.extend(extract_assignment_commands(&combined, "latest_timeline="));
    commands.extend(extract_assignment_commands(&combined, "failed_timeline="));
    commands.extend(extract_assignment_commands(&combined, "replay="));
    commands.extend(extract_assignment_commands(&combined, "page="));
    commands.extend(extract_assignment_commands(&combined, "failed_filter="));
    commands.extend(extract_assignment_commands(&combined, "trace="));
    commands.extend(extract_assignment_commands(&combined, "latest_trace="));
    commands.extend(extract_assignment_commands(&combined, "failed_trace="));
    commands.extend(extract_assignment_commands(&combined, "matrix="));
    commands.extend(extract_assignment_commands(&combined, "fallback="));
    commands.extend(extract_assignment_commands(&combined, "palette="));
    commands.extend(extract_assignment_commands(&combined, "live="));
    commands.extend(extract_assignment_commands(&combined, "health="));
    commands.extend(extract_assignment_commands(&combined, "debug="));
    commands.extend(extract_assignment_commands(&combined, "inspect="));
    commands.extend(extract_assignment_commands(&combined, "probe="));
    commands.extend(extract_assignment_commands(&combined, "readiness="));
    commands.extend(extract_assignment_commands(&combined, "logs="));
    commands.extend(extract_assignment_commands(&combined, "insights="));
    commands.extend(extract_assignment_commands(&combined, "dump="));
    commands.extend(extract_assignment_commands(&combined, "state="));
    commands.extend(extract_assignment_commands(&combined, "memory="));
    commands.extend(extract_assignment_commands(&combined, "context="));
    commands.extend(extract_assignment_commands(&combined, "tools="));
    commands.extend(extract_assignment_commands(&combined, "mcp="));
    commands.extend(extract_assignment_commands(&combined, "stdio="));
    commands.extend(extract_assignment_commands(&combined, "http="));
    commands.extend(extract_assignment_commands(&combined, "browser="));
    commands.extend(extract_assignment_commands(&combined, "launch="));
    commands.extend(extract_assignment_commands(&combined, "supervisor="));
    commands.extend(extract_assignment_commands(&combined, "web="));
    commands.extend(extract_assignment_commands(&combined, "vision="));
    commands.extend(extract_assignment_commands(&combined, "image="));
    commands.extend(extract_assignment_commands(&combined, "generate="));
    commands.extend(extract_assignment_commands(&combined, "list="));
    commands.extend(extract_assignment_commands(&combined, "search="));
    commands.extend(extract_assignment_commands(&combined, "extract="));
    commands.extend(extract_assignment_commands(&combined, "navigate="));
    commands.extend(extract_assignment_commands(&combined, "snapshot="));
    commands.extend(extract_assignment_commands(&combined, "click="));
    commands.extend(extract_assignment_commands(&combined, "type="));
    commands.extend(extract_assignment_commands(&combined, "scroll="));
    commands.extend(extract_assignment_commands(&combined, "screenshot="));
    commands.extend(extract_assignment_commands(&combined, "cdp="));
    commands.extend(extract_assignment_commands(&combined, "start="));
    commands.extend(extract_assignment_commands(&combined, "stop="));
    commands.extend(extract_assignment_commands(&combined, "restart="));
    commands.extend(extract_assignment_commands(&combined, "adapters="));
    if commands.is_empty() {
        commands.push(default_cell_command(cell.kind).to_owned());
    }
    dedupe_commands(commands)
}

pub(in crate::chat::workbench::screen) fn dedupe_commands(commands: Vec<String>) -> Vec<String> {
    commands
        .into_iter()
        .fold(Vec::new(), |mut unique, command| {
            if !unique.contains(&command) {
                unique.push(command);
            }
            unique
        })
}
