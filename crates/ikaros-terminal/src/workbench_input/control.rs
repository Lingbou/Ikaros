// SPDX-License-Identifier: GPL-3.0-only

use super::events::{WorkbenchInputAction, WorkbenchInputEvent, parse_workbench_input_event};
use super::state::{WorkbenchInputState, format_workbench_input_state};
use crate::terminal_inline;
pub fn handle_workbench_input_control(
    input: &str,
    state: &mut WorkbenchInputState,
    quiet: bool,
) -> bool {
    match parse_workbench_input_event(input) {
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistoryPrevious) => {
            if let Some(selected) = state.apply(WorkbenchInputAction::HistoryPrevious) {
                if !quiet {
                    println!("input_history_selected: {}", terminal_inline(&selected));
                }
            } else if !quiet {
                println!("input_history_selected: none");
            }
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistoryNext) => {
            if let Some(selected) = state.apply(WorkbenchInputAction::HistoryNext) {
                if !quiet {
                    println!("input_history_selected: {}", terminal_inline(&selected));
                }
            } else if !quiet {
                println!("input_history_selected: none");
            }
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistorySearchStart) => {
            print_input_edit_state(
                "history_search_start",
                state,
                WorkbenchInputAction::HistorySearchStart,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistorySearchPrevious) => {
            print_input_edit_state(
                "history_search_previous",
                state,
                WorkbenchInputAction::HistorySearchPrevious,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::HistorySearchNext) => {
            print_input_edit_state(
                "history_search_next",
                state,
                WorkbenchInputAction::HistorySearchNext,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveLeft) => {
            print_input_edit_state("move_left", state, WorkbenchInputAction::MoveLeft, quiet);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveRight) => {
            print_input_edit_state("move_right", state, WorkbenchInputAction::MoveRight, quiet);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveWordLeft) => {
            print_input_edit_state(
                "move_word_left",
                state,
                WorkbenchInputAction::MoveWordLeft,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveWordRight) => {
            print_input_edit_state(
                "move_word_right",
                state,
                WorkbenchInputAction::MoveWordRight,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveStart) => {
            print_input_edit_state("move_start", state, WorkbenchInputAction::MoveStart, quiet);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::MoveEnd) => {
            print_input_edit_state("move_end", state, WorkbenchInputAction::MoveEnd, quiet);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeletePrevious) => {
            print_input_edit_state(
                "delete_previous",
                state,
                WorkbenchInputAction::DeletePrevious,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteNext) => {
            print_input_edit_state(
                "delete_next",
                state,
                WorkbenchInputAction::DeleteNext,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeletePreviousWord) => {
            print_input_edit_state(
                "delete_previous_word",
                state,
                WorkbenchInputAction::DeletePreviousWord,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteNextWord) => {
            print_input_edit_state(
                "delete_next_word",
                state,
                WorkbenchInputAction::DeleteNextWord,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteBeforeCursor) => {
            print_input_edit_state(
                "delete_before_cursor",
                state,
                WorkbenchInputAction::DeleteBeforeCursor,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteAfterCursor) => {
            print_input_edit_state(
                "delete_after_cursor",
                state,
                WorkbenchInputAction::DeleteAfterCursor,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::Undo) => {
            print_input_edit_state("undo", state, WorkbenchInputAction::Undo, quiet);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::Redo) => {
            print_input_edit_state("redo", state, WorkbenchInputAction::Redo, quiet);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::Refresh) => {
            print_input_edit_state("refresh", state, WorkbenchInputAction::Refresh, quiet);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::Complete) => {
            print_input_edit_state("complete", state, WorkbenchInputAction::Complete, quiet);
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::CompletionPrevious) => {
            print_input_edit_state(
                "completion_previous",
                state,
                WorkbenchInputAction::CompletionPrevious,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::CompletionNext) => {
            print_input_edit_state(
                "completion_next",
                state,
                WorkbenchInputAction::CompletionNext,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::CompletionPagePrevious) => {
            print_input_edit_state(
                "completion_page_previous",
                state,
                WorkbenchInputAction::CompletionPagePrevious,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::Action(WorkbenchInputAction::CompletionPageNext) => {
            print_input_edit_state(
                "completion_page_next",
                state,
                WorkbenchInputAction::CompletionPageNext,
                quiet,
            );
            true
        }
        WorkbenchInputEvent::CompletePrefix(prefix) => {
            state.set_buffer("");
            state.insert_text(&prefix);
            if let Some(completed) = state.apply(WorkbenchInputAction::Complete) {
                if !quiet {
                    println!("input_completion: {}", terminal_inline(&completed));
                }
            } else if !quiet {
                println!("input_completion: none");
            }
            true
        }
        WorkbenchInputEvent::SubmitLine(_) => false,
    }
}

fn print_input_edit_state(
    action: &str,
    state: &mut WorkbenchInputState,
    input_action: WorkbenchInputAction,
    quiet: bool,
) {
    let buffer = state.apply(input_action).unwrap_or_default();
    if quiet {
        return;
    }
    println!(
        "input_edit: action={} cursor={} buffer={}",
        action,
        state.cursor(),
        terminal_inline(&buffer)
    );
    println!("{}", format_workbench_input_state(action, state));
}
