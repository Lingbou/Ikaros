// SPDX-License-Identifier: GPL-3.0-only

use super::state::WorkbenchInputState;
use crate::{terminal_inline, terminal_message};
use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkbenchInputAction {
    HistoryPrevious,
    HistoryNext,
    #[allow(dead_code)]
    HistorySearchStart,
    HistorySearchPrevious,
    HistorySearchNext,
    Complete,
    CompletionPrevious,
    CompletionNext,
    CompletionPagePrevious,
    CompletionPageNext,
    MoveLeft,
    MoveRight,
    MoveWordLeft,
    MoveWordRight,
    MoveStart,
    MoveEnd,
    DeletePrevious,
    DeleteNext,
    DeletePreviousWord,
    DeleteNextWord,
    DeleteBeforeCursor,
    DeleteAfterCursor,
    Undo,
    Redo,
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchInputEvent {
    Action(WorkbenchInputAction),
    CompletePrefix(String),
    SubmitLine(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchTerminalInputEvent {
    Action(WorkbenchInputAction),
    InsertText(String),
    InsertNewline,
    ClearSession,
    Submit,
    Escape,
    Interrupt,
    EndOfInput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkbenchTerminalInputOutcome {
    Pending,
    Submit(String),
    Exit,
}

pub fn apply_workbench_terminal_input_event(
    state: &mut WorkbenchInputState,
    event: WorkbenchTerminalInputEvent,
) -> WorkbenchTerminalInputOutcome {
    match event {
        WorkbenchTerminalInputEvent::Action(action) => {
            state.apply(action);
            WorkbenchTerminalInputOutcome::Pending
        }
        WorkbenchTerminalInputEvent::InsertText(text) => {
            if state.history_search_active() {
                state.insert_history_search_text(&text);
            } else {
                state.insert_text(&text);
            }
            WorkbenchTerminalInputOutcome::Pending
        }
        WorkbenchTerminalInputEvent::InsertNewline => {
            if state.history_search_active() {
                state.accept_history_search();
            } else {
                state.insert_newline();
            }
            WorkbenchTerminalInputOutcome::Pending
        }
        WorkbenchTerminalInputEvent::ClearSession => {
            state.set_buffer("");
            WorkbenchTerminalInputOutcome::Submit("/clear".into())
        }
        WorkbenchTerminalInputEvent::Submit => {
            if state.history_search_active() {
                state.accept_history_search();
            }
            let submitted = terminal_message(state.buffer().trim());
            if submitted.is_empty() {
                WorkbenchTerminalInputOutcome::Pending
            } else {
                state.set_buffer("");
                WorkbenchTerminalInputOutcome::Submit(submitted)
            }
        }
        WorkbenchTerminalInputEvent::Escape => {
            state.cancel_transient_input();
            WorkbenchTerminalInputOutcome::Pending
        }
        WorkbenchTerminalInputEvent::Interrupt => {
            if state.cancel_transient_input() {
                WorkbenchTerminalInputOutcome::Pending
            } else if state.buffer_is_empty() {
                WorkbenchTerminalInputOutcome::Exit
            } else {
                state.set_buffer("");
                WorkbenchTerminalInputOutcome::Pending
            }
        }
        WorkbenchTerminalInputEvent::EndOfInput => {
            if state.buffer().is_empty() && !state.history_search_active() {
                WorkbenchTerminalInputOutcome::Exit
            } else {
                state.apply(WorkbenchInputAction::DeleteNext);
                WorkbenchTerminalInputOutcome::Pending
            }
        }
    }
}

pub fn parse_workbench_terminal_key_event(event: KeyEvent) -> Option<WorkbenchTerminalInputEvent> {
    if event.kind != KeyEventKind::Press {
        return None;
    }
    if event
        .modifiers
        .intersects(KeyModifiers::SUPER | KeyModifiers::HYPER | KeyModifiers::META)
    {
        return None;
    }
    let control = event.modifiers.contains(KeyModifiers::CONTROL);
    let alt = event.modifiers.contains(KeyModifiers::ALT);
    match event.code {
        KeyCode::Up => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::CompletionPrevious,
        )),
        KeyCode::Down => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::CompletionNext,
        )),
        KeyCode::PageUp => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::CompletionPagePrevious,
        )),
        KeyCode::PageDown => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::CompletionPageNext,
        )),
        KeyCode::Left if control || alt => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveWordLeft,
        )),
        KeyCode::Right if control || alt => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveWordRight,
        )),
        KeyCode::Left => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveLeft,
        )),
        KeyCode::Right => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveRight,
        )),
        KeyCode::Home => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveStart,
        )),
        KeyCode::End => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveEnd,
        )),
        KeyCode::Backspace if control || alt => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::DeletePreviousWord,
        )),
        KeyCode::Backspace => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::DeletePrevious,
        )),
        KeyCode::Delete => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::DeleteNext,
        )),
        KeyCode::Tab | KeyCode::BackTab => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::Complete,
        )),
        KeyCode::Enter if alt || event.modifiers.contains(KeyModifiers::SHIFT) => {
            Some(WorkbenchTerminalInputEvent::InsertNewline)
        }
        KeyCode::Enter => Some(WorkbenchTerminalInputEvent::Submit),
        KeyCode::Esc => Some(WorkbenchTerminalInputEvent::Escape),
        KeyCode::Char('c' | 'C') if control => Some(WorkbenchTerminalInputEvent::Interrupt),
        KeyCode::Char('j' | 'J') if control => Some(WorkbenchTerminalInputEvent::InsertNewline),
        KeyCode::Char('a') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveStart,
        )),
        KeyCode::Char('e') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveEnd,
        )),
        KeyCode::Char('p') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::HistoryPrevious,
        )),
        KeyCode::Char('n') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::HistoryNext,
        )),
        KeyCode::Char('r') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::HistorySearchPrevious,
        )),
        KeyCode::Char('s') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::HistorySearchNext,
        )),
        KeyCode::Char('b') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveLeft,
        )),
        KeyCode::Char('f') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveRight,
        )),
        KeyCode::Char('b') if alt => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveWordLeft,
        )),
        KeyCode::Char('f') if alt => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::MoveWordRight,
        )),
        KeyCode::Char('h') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::DeletePrevious,
        )),
        KeyCode::Char('d' | 'D') if control => Some(WorkbenchTerminalInputEvent::EndOfInput),
        KeyCode::Char('w') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::DeletePreviousWord,
        )),
        KeyCode::Char('d') if alt => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::DeleteNextWord,
        )),
        KeyCode::Char('u') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::DeleteBeforeCursor,
        )),
        KeyCode::Char('k') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::DeleteAfterCursor,
        )),
        KeyCode::Char('l') if control => Some(WorkbenchTerminalInputEvent::ClearSession),
        KeyCode::Char('z' | '_') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::Undo,
        )),
        KeyCode::Char('y') if control => Some(WorkbenchTerminalInputEvent::Action(
            WorkbenchInputAction::Redo,
        )),
        KeyCode::Char(ch) if !control && !alt => Some(WorkbenchTerminalInputEvent::InsertText(
            terminal_inline(&ch.to_string()),
        )),
        _ => None,
    }
}

pub fn parse_workbench_terminal_event(
    event: CrosstermEvent,
) -> Option<WorkbenchTerminalInputEvent> {
    match event {
        CrosstermEvent::Key(key) => parse_workbench_terminal_key_event(key),
        CrosstermEvent::Paste(text) => Some(WorkbenchTerminalInputEvent::InsertText(
            terminal_message(&text),
        )),
        _ => None,
    }
}

pub fn parse_workbench_input_event(input: &str) -> WorkbenchInputEvent {
    if let Some(event) = legacy_terminal_key_input(input) {
        match parse_workbench_terminal_key_event(event) {
            Some(WorkbenchTerminalInputEvent::Action(action)) => {
                return WorkbenchInputEvent::Action(action);
            }
            Some(WorkbenchTerminalInputEvent::EndOfInput) => {
                return WorkbenchInputEvent::Action(WorkbenchInputAction::DeleteNext);
            }
            Some(WorkbenchTerminalInputEvent::ClearSession) => {
                return WorkbenchInputEvent::SubmitLine("/clear".into());
            }
            _ => {}
        }
    }
    input
        .strip_suffix('\t')
        .map(|prefix| WorkbenchInputEvent::CompletePrefix(terminal_inline(prefix)))
        .unwrap_or_else(|| WorkbenchInputEvent::SubmitLine(terminal_inline(input)))
}

fn legacy_terminal_key_input(input: &str) -> Option<KeyEvent> {
    let event = match input {
        "\u{1b}[A" => KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        "\u{1b}[B" => KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        "\u{1b}[D" => KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        "\u{1b}[C" => KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        "\u{1b}[H" | "\u{1b}[1~" => KeyEvent::new(KeyCode::Home, KeyModifiers::NONE),
        "\u{1b}[F" | "\u{1b}[4~" => KeyEvent::new(KeyCode::End, KeyModifiers::NONE),
        "\u{1b}[5~" => KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE),
        "\u{1b}[6~" => KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE),
        "\u{1b}[3~" => KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE),
        "\u{10}" => KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
        "\u{e}" => KeyEvent::new(KeyCode::Char('n'), KeyModifiers::CONTROL),
        "\u{12}" => KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
        "\u{13}" => KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        "\u{2}" => KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
        "\u{6}" => KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
        "\u{1b}b" => KeyEvent::new(KeyCode::Char('b'), KeyModifiers::ALT),
        "\u{1b}f" => KeyEvent::new(KeyCode::Char('f'), KeyModifiers::ALT),
        "\u{1b}d" => KeyEvent::new(KeyCode::Char('d'), KeyModifiers::ALT),
        "\u{1}" => KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL),
        "\u{5}" => KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL),
        "\u{7f}" | "\u{8}" => KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        "\u{4}" => KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
        "\u{17}" => KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL),
        "\u{15}" => KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        "\u{b}" => KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL),
        "\u{c}" => KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL),
        "\u{1a}" => KeyEvent::new(KeyCode::Char('z'), KeyModifiers::CONTROL),
        "\u{1f}" => KeyEvent::new(KeyCode::Char('_'), KeyModifiers::CONTROL),
        "\u{19}" => KeyEvent::new(KeyCode::Char('y'), KeyModifiers::CONTROL),
        _ => return None,
    };
    Some(event)
}
