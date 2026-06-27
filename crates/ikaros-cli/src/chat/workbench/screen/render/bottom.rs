// SPDX-License-Identifier: GPL-3.0-only

use super::*;

pub(in crate::chat::workbench::screen) fn bottom_pane_text(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> String {
    if state.raw_mode() {
        return raw_bottom_pane_text(screen, state);
    }
    human_bottom_pane_text(screen, state)
}

pub(in crate::chat::workbench::screen) fn raw_bottom_pane_text(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> String {
    let bottom = screen
        .status
        .iter()
        .find(|cell| cell.title == "bottom pane");
    let active_view = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "active_view="))
        .unwrap_or_else(|| "composer".into());
    let input_view = extract_assignment_span(&screen.input_hint, "view=", &[" undo="])
        .unwrap_or_else(|| terminal_inline(&screen.input_hint));
    let completion = extract_assignment_span(
        &screen.input_hint,
        "completion_active=",
        &[" completion_candidates="],
    )
    .unwrap_or_else(|| "inactive".into());
    let history = extract_assignment_span(&screen.input_hint, "history_search=", &[])
        .unwrap_or_else(|| "inactive".into());
    [
        status_line_compact_text(screen),
        bottom_pane_progress_line(screen),
        bottom_pane_popup_line(screen, state),
        bottom_pane_surface_line(screen, bottom, &active_view, &input_view),
        bottom_pane_action_menu_line(screen, state),
        format!(
            "composer retained=true visible={} completion={} history={} palette=/screen --palette",
            active_view == "composer",
            completion,
            history,
        ),
        format!(
            "selected {} {}",
            state.footer_summary(),
            selected_action_footer(screen, state),
        ),
        "keys=tab focus alt-1/2/3/4 panels arrows/pgup/pgdn/home navigate enter open alt-enter confirm esc clear-selection help=f1 palette=f5 action=alt-m/r/o/q interrupt=alt-i timeline=ctrl-t all render=f2/f3/f4 alt+a/d approve/deny alt+c cancel alt+x clear ctrl+c interrupt".into(),
    ]
    .join("\n")
}

pub(in crate::chat::workbench::screen) fn human_bottom_pane_text(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> String {
    let bottom = screen
        .status
        .iter()
        .find(|cell| cell.title == "bottom pane");
    let active_view = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "active_view="))
        .unwrap_or_else(|| "composer".into());
    let input_view = extract_assignment_span(&screen.input_hint, "view=", &[" undo="])
        .unwrap_or_else(|| terminal_inline(&screen.input_hint));

    let mut lines = vec![
        human_status_line(screen),
        human_progress_line(screen),
        human_surface_line(screen, bottom, &active_view, &input_view),
        human_action_menu_line(screen, state),
        "Keys: Enter send/open | F5 commands | F1 help | Esc close panel | Ctrl+C clear or exit"
            .into(),
    ];
    if let Some(popup) = human_popup_line(screen, state) {
        lines.insert(2, popup);
    }
    lines.join("\n")
}

pub(in crate::chat::workbench::screen) fn human_status_line(screen: &WorkbenchScreen) -> String {
    let model = find_cell(screen, |cell| cell.title == "model");
    let budget = find_cell(screen, |cell| cell.title == "model budget");
    let provider_health = find_cell(screen, |cell| cell.title == "provider health");
    let provider_recovery = find_cell(screen, |cell| cell.title == "provider recovery");
    let queue = find_cell(screen, |cell| cell.title == "queue");
    let bottom = find_cell(screen, |cell| cell.title == "bottom pane");
    let progress = find_cell(screen, |cell| cell.title == "progress");
    let provider = model
        .and_then(|cell| extract_token_after(&cell.detail, "provider="))
        .unwrap_or_else(|| "unknown".into());
    let model_name = model
        .and_then(|cell| extract_token_after(&cell.detail, "model="))
        .unwrap_or_else(|| "unknown".into());
    let budget_status = budget
        .and_then(|cell| extract_token_after(&cell.detail, "budget_status="))
        .unwrap_or_else(|| "unknown".into());
    let provider_status = provider_health
        .and_then(|cell| extract_token_after(&cell.detail, "health_status="))
        .or_else(|| provider_recovery.and_then(|cell| extract_token_after(&cell.detail, "status=")))
        .unwrap_or_else(|| "unknown".into());
    let approvals = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "approvals="))
        .unwrap_or_else(|| "0".into());
    let queued = queue
        .and_then(|cell| extract_token_after(&cell.detail, "queued="))
        .unwrap_or_else(|| "0".into());
    let running = queue
        .and_then(|cell| extract_token_after(&cell.detail, "running="))
        .unwrap_or_else(|| "0".into());
    let failed = queue
        .and_then(|cell| extract_token_after(&cell.detail, "failed="))
        .unwrap_or_else(|| "0".into());
    let progress_status = progress
        .and_then(|cell| extract_token_after(&cell.detail, "status="))
        .unwrap_or_else(|| "idle".into());
    let activity = match progress_status.as_str() {
        "running" => "Running",
        "approval_pending" => "Waiting for approval",
        "failed" => "Needs recovery",
        "completed" => "Ready",
        _ => "Ready",
    };
    let queue_text = if queued == "0" && running == "0" && failed == "0" {
        "Queue empty".into()
    } else {
        format!("Queue {queued} waiting, {running} running, {failed} failed")
    };
    let approvals_text = if approvals == "0" {
        "No approvals pending".into()
    } else {
        format!("{approvals} approval request(s)")
    };
    let budget_text = if budget_status == "unknown" {
        "Budget unknown".into()
    } else {
        format!("Budget {budget_status}")
    };
    format!(
        "{activity}. Model {}/{}. Provider {}. {budget_text}. {approvals_text}. {queue_text}.",
        terminal_inline(&provider),
        terminal_inline(&model_name),
        terminal_inline(&provider_status),
    )
}

pub(in crate::chat::workbench::screen) fn human_progress_line(screen: &WorkbenchScreen) -> String {
    let progress = screen_surface_progress_json(screen);
    let status = json_string(&progress, "status", "idle");
    let phase = json_string(&progress, "phase", "idle");
    let detail = json_string(&progress, "detail", "none");
    match status.as_str() {
        "idle" => "No active turn.".into(),
        "running" => format!("Running: {phase}. {}", short_status_detail(&detail)),
        "approval_pending" => "Waiting for approval. Review the request in the side panel.".into(),
        "failed" => format!("Turn failed. {}", short_status_detail(&detail)),
        "completed" => "Last turn completed.".into(),
        other => format!(
            "Turn status: {}. {}",
            terminal_inline(other),
            short_status_detail(&detail)
        ),
    }
}

pub(in crate::chat::workbench::screen) fn human_popup_line(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> Option<String> {
    let popup = screen_input_popup_json(screen, state);
    let kind = popup
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or("none");
    match kind {
        "history_search" => Some(format!(
            "History search: {} match(es). Enter accepts; Esc closes.",
            json_string(&popup, "matches", "0"),
        )),
        "command_completion" => Some(format!(
            "Command completion: {} candidate(s). Tab cycles; Enter accepts.",
            popup
                .get("completion_items")
                .and_then(|value| value.as_array())
                .map(Vec::len)
                .unwrap_or_default(),
        )),
        "command_palette" => Some(format!(
            "Command palette: {} match(es). Type to filter; Enter opens.",
            popup
                .get("palette_items")
                .and_then(|value| value.as_array())
                .map(Vec::len)
                .unwrap_or_default(),
        )),
        _ => None,
    }
}

pub(in crate::chat::workbench::screen) fn human_surface_line(
    screen: &WorkbenchScreen,
    bottom: Option<&WorkbenchCell>,
    active_view: &str,
    input_view: &str,
) -> String {
    match active_view {
        "approval" => {
            let approvals = bottom
                .and_then(|cell| extract_token_after(&cell.detail, "approvals="))
                .unwrap_or_else(|| "0".into());
            format!(
                "Approval review open. {approvals} pending. Enter inspects; Alt+A approves; Alt+D denies."
            )
        }
        "input_queue" => {
            let count = bottom
                .and_then(|cell| extract_token_after(&cell.detail, "pending_inputs="))
                .unwrap_or_else(|| "0".into());
            format!(
                "Queued input waiting. {count} pending. Enter runs the queue; Alt+X clears selected."
            )
        }
        "attachments" => {
            let count = bottom
                .and_then(|cell| extract_token_after(&cell.detail, "attachments="))
                .unwrap_or_else(|| "0".into());
            format!(
                "Attachments staged. {count} pending. Open attachments to review or clear them."
            )
        }
        "continuation" => {
            let count = bottom
                .and_then(|cell| extract_token_after(&cell.detail, "continuations="))
                .unwrap_or_else(|| "0".into());
            format!("Continuation queue active. {count} item(s). Enter opens queue controls.")
        }
        _ => composer_surface_line(screen, input_view),
    }
}

pub(in crate::chat::workbench::screen) fn composer_surface_line(
    screen: &WorkbenchScreen,
    input_view: &str,
) -> String {
    let input = terminal_inline(input_view).trim().to_owned();
    if input.is_empty()
        || input.contains('=')
        || input.starts_with("type")
        || input == terminal_inline(&screen.input_hint)
    {
        "Composer ready. Type a message or slash command.".into()
    } else {
        format!("Draft: {}", truncate_terminal_text(&input, 96))
    }
}

pub(in crate::chat::workbench::screen) fn human_action_menu_line(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> String {
    let action_menu = bottom_pane_action_menu_model(screen, state);
    let primary_label = action_menu
        .get("primary")
        .and_then(|value| value.get("label"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("No action");
    let shortcut = action_menu
        .get("primary")
        .and_then(|value| value.get("shortcut"))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .unwrap_or("Enter");
    format!(
        "Action: {} ({shortcut}). Alt+Enter confirms selected actions.",
        terminal_inline(primary_label),
    )
}

pub(in crate::chat::workbench::screen) fn bottom_pane_popup_line(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> String {
    let popup = screen_input_popup_json(screen, state);
    let kind = popup
        .get("kind")
        .and_then(|value| value.as_str())
        .unwrap_or("none");
    match kind {
        "history_search" => format!(
            "popup=history_search query={} matches={} selected={} accept=enter cancel=esc",
            json_string(&popup, "query", "none"),
            json_string(&popup, "matches", "0"),
            json_string(&popup, "selected_index", "0/0"),
        ),
        "command_completion" | "command_palette" => format!(
            "popup={} query={} completion_items={} palette_items={} accept=enter cycle=tab inspect=/commands",
            kind,
            json_string(&popup, "query", "all"),
            popup
                .get("completion_items")
                .and_then(|value| value.as_array())
                .map(Vec::len)
                .unwrap_or_default(),
            popup
                .get("palette_items")
                .and_then(|value| value.as_array())
                .map(Vec::len)
                .unwrap_or_default(),
        ),
        _ => "popup=none".into(),
    }
}

pub(in crate::chat::workbench::screen) fn bottom_pane_action_menu_line(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> String {
    let action_menu = bottom_pane_action_menu_model(screen, state);
    let primary_label = action_menu
        .get("primary")
        .and_then(|value| value.get("label"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("No action");
    let primary_command = action_menu
        .get("primary")
        .and_then(|value| value.get("command"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let selector = action_menu
        .get("selection_selector")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("none");
    let groups = action_menu
        .get("groups")
        .and_then(serde_json::Value::as_array)
        .map(|groups| {
            groups
                .iter()
                .filter_map(|group| {
                    let id = group.get("id").and_then(serde_json::Value::as_str)?;
                    let count = group
                        .get("items")
                        .and_then(serde_json::Value::as_array)
                        .map(Vec::len)
                        .unwrap_or_default();
                    (count > 0).then(|| format!("{id}:{count}"))
                })
                .collect::<Vec<_>>()
                .join(",")
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "none".into());
    format!(
        "actions selector={} primary_label={} primary={} groups={} open=enter confirm=alt-enter global_keys=f1:help,f5:palette action_keys=alt-m:primary,alt-r:recovery,alt-o:approval,alt-q:queue,alt-i:interrupt select=/screen --select-action primary timeline_keys=ctrl-t palette=/screen --palette",
        terminal_inline(selector),
        terminal_inline(primary_label),
        terminal_inline(primary_command),
        terminal_inline(&groups),
    )
}

pub(in crate::chat::workbench::screen) fn bottom_pane_action_menu_model(
    screen: &WorkbenchScreen,
    state: &WorkbenchScreenState,
) -> serde_json::Value {
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
    screen_action_menu_model_json(
        screen,
        state,
        &recovery,
        &approval_overlay,
        &input_popup,
        &turn_state,
        &overlay_routing,
    )
}

pub(in crate::chat::workbench::screen) fn bottom_pane_progress_line(
    screen: &WorkbenchScreen,
) -> String {
    let progress = screen_surface_progress_json(screen);
    format!(
        "status={} phase={} spinner={} progress={} detail={}",
        json_string(&progress, "status", "idle"),
        json_string(&progress, "phase", "idle"),
        json_string(&progress, "spinner", "-"),
        json_string(&progress, "progress_bar", "[----------]"),
        json_string(&progress, "detail", "none"),
    )
}

pub(in crate::chat::workbench::screen) fn status_line_compact_text(
    screen: &WorkbenchScreen,
) -> String {
    let model = find_cell(screen, |cell| cell.title == "model");
    let budget = find_cell(screen, |cell| cell.title == "model budget");
    let provider_health = find_cell(screen, |cell| cell.title == "provider health");
    let provider_recovery = find_cell(screen, |cell| cell.title == "provider recovery");
    let queue = find_cell(screen, |cell| cell.title == "queue");
    let bottom = find_cell(screen, |cell| cell.title == "bottom pane");
    let progress = find_cell(screen, |cell| cell.title == "progress");
    let provider = model
        .and_then(|cell| extract_token_after(&cell.detail, "provider="))
        .unwrap_or_else(|| "unknown".into());
    let model_name = model
        .and_then(|cell| extract_token_after(&cell.detail, "model="))
        .unwrap_or_else(|| "unknown".into());
    let budget_status = budget
        .and_then(|cell| extract_token_after(&cell.detail, "budget_status="))
        .unwrap_or_else(|| "unknown".into());
    let provider_status = provider_health
        .and_then(|cell| extract_token_after(&cell.detail, "health_status="))
        .or_else(|| provider_recovery.and_then(|cell| extract_token_after(&cell.detail, "status=")))
        .unwrap_or_else(|| "unknown".into());
    let approvals = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "approvals="))
        .unwrap_or_else(|| "0".into());
    let queued = queue
        .and_then(|cell| extract_token_after(&cell.detail, "queued="))
        .unwrap_or_else(|| "0".into());
    let running = queue
        .and_then(|cell| extract_token_after(&cell.detail, "running="))
        .unwrap_or_else(|| "0".into());
    let failed = queue
        .and_then(|cell| extract_token_after(&cell.detail, "failed="))
        .unwrap_or_else(|| "0".into());
    let progress_status = progress
        .and_then(|cell| extract_token_after(&cell.detail, "status="))
        .unwrap_or_else(|| "idle".into());
    format!(
        "status activity={} model={}/{} provider={} budget={} approvals={} queue=q:{} r:{} f:{} actions=/provider /budget /approval /debug continuations",
        progress_status,
        provider,
        model_name,
        provider_status,
        budget_status,
        approvals,
        queued,
        running,
        failed,
    )
}

pub(in crate::chat::workbench::screen) fn bottom_pane_surface_line(
    screen: &WorkbenchScreen,
    bottom: Option<&WorkbenchCell>,
    active_view: &str,
    input_view: &str,
) -> String {
    match active_view {
        "approval" => bottom_pane_approval_line(screen, bottom),
        "input_queue" => bottom_pane_pending_input_line(bottom),
        "attachments" => bottom_pane_attachment_line(bottom),
        "continuation" => bottom_pane_continuation_line(bottom),
        _ => format!(
            "surface=composer input={} queued_inputs=0 attachments=0",
            input_view
        ),
    }
}

pub(in crate::chat::workbench::screen) fn bottom_pane_approval_line(
    screen: &WorkbenchScreen,
    bottom: Option<&WorkbenchCell>,
) -> String {
    let approvals = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "approvals="))
        .unwrap_or_else(|| "0".into());
    let modal = screen_modal_cell(screen);
    let primary_id = modal
        .and_then(|cell| extract_assignment_span(&cell.detail, "approve=", &[" deny="]))
        .and_then(|command| approval_id_from_decision_command(&command))
        .unwrap_or_else(|| "none".into());
    format!(
        "surface=approval visible=true pending={} primary_id={} approve=/screen approve-selected deny=/screen deny-selected inspect=/approval list",
        approvals, primary_id,
    )
}

pub(in crate::chat::workbench::screen) fn bottom_pane_pending_input_line(
    bottom: Option<&WorkbenchCell>,
) -> String {
    let count = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "pending_inputs="))
        .unwrap_or_else(|| "0".into());
    let next_input = bottom
        .and_then(|cell| extract_assignment_span(&cell.detail, "next_input=", &[" attachments="]))
        .unwrap_or_else(|| "none".into());
    format!(
        "surface=input_queue visible=true pending_inputs={} next={} run=/queue run clear=/queue clear",
        count, next_input,
    )
}

pub(in crate::chat::workbench::screen) fn bottom_pane_attachment_line(
    bottom: Option<&WorkbenchCell>,
) -> String {
    let count = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "attachments="))
        .unwrap_or_else(|| "0".into());
    format!(
        "surface=attachments visible=true pending={} list=/attach list clear=/attach clear",
        count,
    )
}

pub(in crate::chat::workbench::screen) fn bottom_pane_continuation_line(
    bottom: Option<&WorkbenchCell>,
) -> String {
    let count = bottom
        .and_then(|cell| extract_token_after(&cell.detail, "continuations="))
        .unwrap_or_else(|| "0".into());
    format!(
        "surface=continuation visible=true count={} run=/queue run cancel=/cancel all debug=/debug continuations",
        count,
    )
}
