// SPDX-License-Identifier: GPL-3.0-only

use crate::chat::progress::WorkbenchProgressSnapshot;
use crate::chat::workbench::{WorkbenchCell, WorkbenchCellKind, WorkbenchScreen, terminal_message};
use ikaros_providers::model::ModelStreamEvent;
use ikaros_state::session::{AgentEvent, AgentEventKind};
use ikaros_terminal::screen_progress_status_cell;

use super::progress_budget_recovery_command;

pub(in crate::chat) fn apply_progress_to_cached_screen(
    screen: &mut WorkbenchScreen,
    progress: &WorkbenchProgressSnapshot,
) {
    let budget_command = progress_budget_recovery_command(Some(progress));
    let progress_cell = screen_progress_status_cell(Some(progress), budget_command.as_deref());
    if let Some(existing) = screen
        .status
        .iter_mut()
        .find(|cell| cell.title == "progress")
    {
        *existing = progress_cell;
    } else {
        screen.status.push(progress_cell);
    }
}

pub(in crate::chat) fn apply_pending_user_input_to_cached_screen(
    screen: &mut WorkbenchScreen,
    input: &str,
) {
    let detail = terminal_message(input.trim());
    if detail.is_empty() {
        return;
    }
    if screen
        .main
        .iter()
        .rev()
        .find(|cell| is_conversation_cell(cell))
        .is_some_and(|cell| cell.title.starts_with("user turn=") && cell.detail == detail)
    {
        return;
    }
    let cell = WorkbenchCell {
        kind: WorkbenchCellKind::Session,
        title: "user turn=pending".into(),
        detail,
    };
    if let Some(existing) = screen
        .main
        .iter_mut()
        .find(|cell| cell.title == "user turn=pending")
    {
        *existing = cell;
        return;
    }
    let insert_at = screen
        .main
        .iter()
        .rposition(is_conversation_cell)
        .map(|index| index + 1)
        .unwrap_or(0);
    screen.main.insert(insert_at, cell);
}

pub(in crate::chat) fn apply_live_model_stream_to_cached_screen(
    screen: &mut WorkbenchScreen,
    events: &[AgentEvent],
) {
    let mut content = String::new();
    let mut has_stream_event = false;
    let mut done = false;
    for event in events {
        let AgentEventKind::ModelStream(stream_event) = &event.kind else {
            continue;
        };
        has_stream_event = true;
        match stream_event {
            ModelStreamEvent::TextDelta(text) => content.push_str(text),
            ModelStreamEvent::Done => done = true,
            _ => {}
        }
    }
    if !has_stream_event || content.trim().is_empty() {
        return;
    }
    let title = if done {
        "assistant turn=streaming done"
    } else {
        "assistant turn=streaming"
    };
    let cell = WorkbenchCell {
        kind: WorkbenchCellKind::Model,
        title: title.into(),
        detail: terminal_message(&content),
    };
    if let Some(existing) = screen
        .main
        .iter_mut()
        .find(|cell| cell.title.starts_with("assistant turn=streaming"))
    {
        *existing = cell;
        return;
    }
    let insert_at = screen
        .main
        .iter()
        .rposition(is_conversation_cell)
        .map(|index| index + 1)
        .unwrap_or(0);
    screen.main.insert(insert_at, cell);
}

fn is_conversation_cell(cell: &WorkbenchCell) -> bool {
    cell.title.starts_with("user turn=") || cell.title.starts_with("assistant turn=")
}
