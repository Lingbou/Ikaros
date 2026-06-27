// SPDX-License-Identifier: GPL-3.0-only

use super::{
    slash::{slash_command_completion_candidates, slash_completion_query},
    terminal_inline, terminal_message,
};
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
            let submitted = terminal_message(state.buffer.trim());
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

pub fn format_workbench_input_state(action: &str, state: &WorkbenchInputState) -> String {
    format!(
        "input_state: action={} cursor={} buffer={} view={} undo={} redo={} completion_active={} completion_candidates={} history_search={}",
        terminal_inline(action),
        state.cursor(),
        terminal_inline(&state.buffer),
        input_cursor_view(&state.buffer, state.cursor()),
        state.undo_stack.len(),
        state.redo_stack.len(),
        terminal_inline(&state.completion_active_summary()),
        input_completion_candidates(state),
        terminal_inline(&state.history_search_summary()),
    )
}

fn input_cursor_view(buffer: &str, cursor: usize) -> String {
    let cursor = cursor.min(buffer.chars().count());
    let mut output = String::new();
    for (index, ch) in terminal_inline(buffer).chars().enumerate() {
        if index == cursor {
            output.push('|');
        }
        output.push(ch);
    }
    if cursor == buffer.chars().count() {
        output.push('|');
    }
    output
}

fn input_completion_candidates(state: &WorkbenchInputState) -> String {
    let candidates = state.completion_candidates();
    if candidates.is_empty() {
        "none".into()
    } else {
        terminal_inline(&candidates.join(","))
    }
}

fn previous_word_boundary_in(input: &str) -> usize {
    let chars = input.chars().collect::<Vec<_>>();
    let mut index = chars.len();
    while index > 0 && chars[index - 1].is_whitespace() {
        index -= 1;
    }
    while index > 0 && !chars[index - 1].is_whitespace() {
        index -= 1;
    }
    index
}

fn byte_index_for_char(input: &str, cursor: usize) -> usize {
    if cursor == 0 {
        return 0;
    }
    input
        .char_indices()
        .nth(cursor)
        .map(|(index, _)| index)
        .unwrap_or(input.len())
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkbenchInputState {
    buffer: String,
    cursor: usize,
    history: Vec<String>,
    history_cursor: Option<usize>,
    undo_stack: Vec<(String, usize)>,
    redo_stack: Vec<(String, usize)>,
    completion_cycle: Option<WorkbenchCompletionCycle>,
    history_search: Option<WorkbenchHistorySearch>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct WorkbenchCompletionCycle {
    query: String,
    candidates: Vec<String>,
    index: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct WorkbenchHistorySearch {
    query: String,
    matches: Vec<usize>,
    index: usize,
    original_buffer: String,
    original_cursor: usize,
}

impl WorkbenchInputState {
    pub fn from_history(entries: impl IntoIterator<Item = String>) -> Self {
        let mut state = Self::default();
        for entry in entries {
            state.record_history(&entry);
        }
        state
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn cursor_view(&self) -> String {
        input_cursor_view(&self.buffer, self.cursor)
    }

    pub fn buffer_is_empty(&self) -> bool {
        self.buffer.trim().is_empty()
    }

    pub fn set_buffer(&mut self, input: &str) {
        self.buffer = terminal_message(input);
        self.cursor = self.buffer.chars().count();
        self.history_cursor = None;
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.clear_completion_cycle();
        self.clear_history_search();
    }

    pub fn insert_text(&mut self, input: &str) {
        let input = terminal_inline(input);
        if input.is_empty() {
            return;
        }
        self.push_undo();
        self.clear_completion_cycle();
        self.clear_history_search();
        let byte_index = self.byte_index_for_cursor(self.cursor);
        self.buffer.insert_str(byte_index, &input);
        self.cursor += input.chars().count();
        let cleaned = terminal_message(&self.buffer);
        if cleaned != self.buffer {
            self.buffer = cleaned;
            self.cursor = self.buffer.chars().count();
        }
        self.history_cursor = None;
    }

    pub fn insert_newline(&mut self) {
        self.push_undo();
        self.clear_completion_cycle();
        self.clear_history_search();
        let byte_index = self.byte_index_for_cursor(self.cursor);
        self.buffer.insert(byte_index, '\n');
        self.cursor += 1;
        self.history_cursor = None;
    }

    pub fn record_history(&mut self, input: &str) {
        let input = terminal_message(input.trim());
        if input.is_empty() {
            return;
        }
        self.history.push(input);
        self.history_cursor = None;
    }

    pub fn history_search_active(&self) -> bool {
        self.history_search.is_some()
    }

    pub fn history_search_summary(&self) -> String {
        let Some(search) = &self.history_search else {
            return "none".into();
        };
        let selected = search
            .matches
            .get(search.index)
            .and_then(|history_index| self.history.get(*history_index))
            .map(String::as_str)
            .unwrap_or("none");
        format!(
            "query={} matches={} selected_index={}/{} selected={}",
            terminal_inline(&search.query),
            search.matches.len(),
            if search.matches.is_empty() {
                0
            } else {
                search.index.saturating_add(1)
            },
            search.matches.len(),
            terminal_inline(selected),
        )
    }

    pub fn history_search_candidates(&self, limit: usize) -> Vec<String> {
        let Some(search) = &self.history_search else {
            return Vec::new();
        };
        search
            .matches
            .iter()
            .filter_map(|history_index| self.history.get(*history_index))
            .take(limit.max(1))
            .cloned()
            .collect()
    }

    pub fn insert_history_search_text(&mut self, input: &str) {
        self.start_history_search_if_needed();
        let input = terminal_inline(input);
        if input.is_empty() {
            return;
        }
        if let Some(search) = &mut self.history_search {
            search.query.push_str(&input);
        }
        self.refresh_history_search_matches();
    }

    pub fn accept_history_search(&mut self) {
        if self.history_search.is_none() {
            return;
        }
        self.history_search = None;
        self.history_cursor = None;
        self.clear_completion_cycle();
    }

    pub fn cancel_history_search(&mut self) {
        let Some(search) = self.history_search.take() else {
            return;
        };
        self.buffer = search.original_buffer;
        self.cursor = search.original_cursor.min(self.buffer.chars().count());
        self.history_cursor = None;
        self.clear_completion_cycle();
    }

    pub fn cancel_transient_input(&mut self) -> bool {
        if self.history_search.is_some() {
            self.cancel_history_search();
            return true;
        }
        let had_completion = self.completion_cycle.is_some();
        self.clear_completion_cycle();
        had_completion
    }
    pub fn history_entries(&self) -> &[String] {
        &self.history
    }

    pub fn completion_candidates(&self) -> Vec<String> {
        if let Some(cycle) = &self.completion_cycle {
            return cycle.candidates.clone();
        }
        slash_command_completion_candidates(&self.buffer, 12)
            .into_iter()
            .map(|candidate| candidate.name.to_owned())
            .collect()
    }

    pub fn completion_query(&self) -> String {
        self.completion_cycle
            .as_ref()
            .map(|cycle| cycle.query.clone())
            .unwrap_or_else(|| slash_completion_query(&self.buffer).to_owned())
    }

    pub fn completion_active_summary(&self) -> String {
        let Some(cycle) = &self.completion_cycle else {
            return "none".into();
        };
        let selected = self.completion_selected().unwrap_or("none");
        format!(
            "query={} selected={} index={}/{}",
            terminal_inline(&cycle.query),
            terminal_inline(selected),
            cycle.index.saturating_add(1),
            cycle.candidates.len()
        )
    }

    pub fn completion_selected(&self) -> Option<&str> {
        let cycle = self.completion_cycle.as_ref()?;
        cycle.candidates.get(cycle.index).map(String::as_str)
    }

    pub fn apply(&mut self, action: WorkbenchInputAction) -> Option<String> {
        if self.history_search_active() {
            return self.apply_history_search_action(action);
        }
        match action {
            WorkbenchInputAction::HistoryPrevious => self.history_previous(),
            WorkbenchInputAction::HistoryNext => self.history_next(),
            WorkbenchInputAction::HistorySearchStart => self.history_search_previous(),
            WorkbenchInputAction::HistorySearchPrevious => self.history_search_previous(),
            WorkbenchInputAction::HistorySearchNext => self.history_search_next(),
            WorkbenchInputAction::Complete => self.complete(),
            WorkbenchInputAction::CompletionPrevious => self
                .select_completion_previous()
                .or_else(|| self.history_previous()),
            WorkbenchInputAction::CompletionNext => self
                .select_completion_next()
                .or_else(|| self.history_next()),
            WorkbenchInputAction::CompletionPagePrevious => self
                .select_completion_page_previous()
                .or_else(|| self.history_previous()),
            WorkbenchInputAction::CompletionPageNext => self
                .select_completion_page_next()
                .or_else(|| self.history_next()),
            WorkbenchInputAction::MoveLeft => {
                self.cursor = self.cursor.saturating_sub(1);
                Some(self.buffer.clone())
            }
            WorkbenchInputAction::MoveRight => {
                self.cursor = (self.cursor + 1).min(self.buffer.chars().count());
                Some(self.buffer.clone())
            }
            WorkbenchInputAction::MoveWordLeft => {
                self.cursor = self.previous_word_boundary();
                Some(self.buffer.clone())
            }
            WorkbenchInputAction::MoveWordRight => {
                self.cursor = self.next_word_boundary();
                Some(self.buffer.clone())
            }
            WorkbenchInputAction::MoveStart => {
                self.cursor = 0;
                Some(self.buffer.clone())
            }
            WorkbenchInputAction::MoveEnd => {
                self.cursor = self.buffer.chars().count();
                Some(self.buffer.clone())
            }
            WorkbenchInputAction::DeletePrevious => self.delete_previous(),
            WorkbenchInputAction::DeleteNext => self.delete_next(),
            WorkbenchInputAction::DeletePreviousWord => self.delete_previous_word(),
            WorkbenchInputAction::DeleteNextWord => self.delete_next_word(),
            WorkbenchInputAction::DeleteBeforeCursor => self.delete_before_cursor(),
            WorkbenchInputAction::DeleteAfterCursor => self.delete_after_cursor(),
            WorkbenchInputAction::Undo => self.undo(),
            WorkbenchInputAction::Redo => self.redo(),
            WorkbenchInputAction::Refresh => Some(self.buffer.clone()),
        }
    }

    fn apply_history_search_action(&mut self, action: WorkbenchInputAction) -> Option<String> {
        match action {
            WorkbenchInputAction::HistoryPrevious | WorkbenchInputAction::HistorySearchPrevious => {
                self.history_search_previous()
            }
            WorkbenchInputAction::HistoryNext | WorkbenchInputAction::HistorySearchNext => {
                self.history_search_next()
            }
            WorkbenchInputAction::DeletePrevious => self.delete_history_search_previous(),
            WorkbenchInputAction::DeletePreviousWord => self.delete_history_search_previous_word(),
            WorkbenchInputAction::DeleteBeforeCursor => self.clear_history_search_query(),
            WorkbenchInputAction::Undo => {
                self.cancel_history_search();
                Some(self.buffer.clone())
            }
            WorkbenchInputAction::Redo
            | WorkbenchInputAction::HistorySearchStart
            | WorkbenchInputAction::Complete
            | WorkbenchInputAction::CompletionPrevious
            | WorkbenchInputAction::CompletionNext
            | WorkbenchInputAction::CompletionPagePrevious
            | WorkbenchInputAction::CompletionPageNext
            | WorkbenchInputAction::MoveLeft
            | WorkbenchInputAction::MoveRight
            | WorkbenchInputAction::MoveWordLeft
            | WorkbenchInputAction::MoveWordRight
            | WorkbenchInputAction::MoveStart
            | WorkbenchInputAction::MoveEnd
            | WorkbenchInputAction::DeleteNext
            | WorkbenchInputAction::DeleteNextWord
            | WorkbenchInputAction::DeleteAfterCursor
            | WorkbenchInputAction::Refresh => Some(self.buffer.clone()),
        }
    }

    fn history_previous(&mut self) -> Option<String> {
        if self.history.is_empty() {
            return None;
        }
        let cursor = self
            .history_cursor
            .map(|cursor| cursor.saturating_sub(1))
            .unwrap_or_else(|| self.history.len().saturating_sub(1));
        self.history_cursor = Some(cursor);
        self.buffer = self.history[cursor].clone();
        self.cursor = self.buffer.chars().count();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.clear_completion_cycle();
        self.clear_history_search();
        Some(self.buffer.clone())
    }

    fn history_next(&mut self) -> Option<String> {
        let cursor = self.history_cursor?;
        if cursor + 1 >= self.history.len() {
            self.history_cursor = None;
            self.buffer.clear();
            self.cursor = 0;
            self.undo_stack.clear();
            self.redo_stack.clear();
            self.clear_completion_cycle();
            self.clear_history_search();
            return Some(String::new());
        }
        let cursor = cursor + 1;
        self.history_cursor = Some(cursor);
        self.buffer = self.history[cursor].clone();
        self.cursor = self.buffer.chars().count();
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.clear_completion_cycle();
        self.clear_history_search();
        Some(self.buffer.clone())
    }

    fn history_search_previous(&mut self) -> Option<String> {
        self.start_history_search_if_needed();
        self.cycle_history_search(1)
    }

    fn history_search_next(&mut self) -> Option<String> {
        self.start_history_search_if_needed();
        self.cycle_history_search_reverse()
    }

    fn delete_history_search_previous(&mut self) -> Option<String> {
        let Some(search) = &mut self.history_search else {
            return None;
        };
        search.query.pop();
        self.refresh_history_search_matches();
        Some(self.buffer.clone())
    }

    fn clear_history_search_query(&mut self) -> Option<String> {
        let Some(search) = &mut self.history_search else {
            return None;
        };
        search.query.clear();
        self.refresh_history_search_matches();
        Some(self.buffer.clone())
    }

    fn delete_history_search_previous_word(&mut self) -> Option<String> {
        let Some(search) = &mut self.history_search else {
            return None;
        };
        let keep = previous_word_boundary_in(&search.query);
        search
            .query
            .truncate(byte_index_for_char(&search.query, keep));
        self.refresh_history_search_matches();
        Some(self.buffer.clone())
    }

    fn complete(&mut self) -> Option<String> {
        self.ensure_completion_cycle()?;
        let selected = self
            .completion_cycle
            .as_ref()
            .and_then(|cycle| cycle.candidates.get(cycle.index))
            .cloned()?;
        self.push_undo();
        self.replace_completion_token(&selected, true);
        self.clear_completion_cycle();
        Some(self.buffer.clone())
    }

    fn select_completion_previous(&mut self) -> Option<String> {
        self.ensure_completion_cycle()?;
        let cycle = self.completion_cycle.as_mut()?;
        cycle.index = if cycle.index == 0 {
            cycle.candidates.len().saturating_sub(1)
        } else {
            cycle.index - 1
        };
        Some(self.buffer.clone())
    }

    fn select_completion_next(&mut self) -> Option<String> {
        self.ensure_completion_cycle()?;
        let cycle = self.completion_cycle.as_mut()?;
        cycle.index = (cycle.index + 1) % cycle.candidates.len();
        Some(self.buffer.clone())
    }

    fn select_completion_page_previous(&mut self) -> Option<String> {
        self.ensure_completion_cycle()?;
        let cycle = self.completion_cycle.as_mut()?;
        cycle.index = cycle.index.saturating_sub(5);
        Some(self.buffer.clone())
    }

    fn select_completion_page_next(&mut self) -> Option<String> {
        self.ensure_completion_cycle()?;
        let cycle = self.completion_cycle.as_mut()?;
        cycle.index = cycle
            .index
            .saturating_add(5)
            .min(cycle.candidates.len().saturating_sub(1));
        Some(self.buffer.clone())
    }

    fn ensure_completion_cycle(&mut self) -> Option<()> {
        let query = self.completion_query();
        if query.is_empty() {
            return None;
        }
        let current_candidates = slash_command_completion_candidates(&query, 24)
            .into_iter()
            .map(|candidate| candidate.name.to_owned())
            .collect::<Vec<_>>();
        if current_candidates.is_empty() {
            self.clear_completion_cycle();
            return None;
        }
        let current_token = self.current_completion_token()?;
        let reuse = self.completion_cycle.as_ref().is_some_and(|cycle| {
            cycle.query == query
                && cycle.candidates == current_candidates
                && cycle
                    .candidates
                    .get(cycle.index)
                    .is_some_and(|selected| selected == &current_token || current_token == query)
        });
        if !reuse {
            self.completion_cycle = Some(WorkbenchCompletionCycle {
                query,
                candidates: current_candidates,
                index: 0,
            });
        }
        Some(())
    }

    fn delete_previous(&mut self) -> Option<String> {
        if self.cursor == 0 {
            return Some(self.buffer.clone());
        }
        self.push_undo();
        self.clear_completion_cycle();
        self.clear_history_search();
        let remove_at = self.cursor - 1;
        self.remove_char_at(remove_at);
        self.cursor = remove_at;
        self.history_cursor = None;
        Some(self.buffer.clone())
    }

    fn delete_next(&mut self) -> Option<String> {
        if self.cursor >= self.buffer.chars().count() {
            return Some(self.buffer.clone());
        }
        self.push_undo();
        self.clear_completion_cycle();
        self.clear_history_search();
        self.remove_char_at(self.cursor);
        self.history_cursor = None;
        Some(self.buffer.clone())
    }

    fn delete_previous_word(&mut self) -> Option<String> {
        let start = self.previous_word_boundary();
        if start == self.cursor {
            return Some(self.buffer.clone());
        }
        self.push_undo();
        self.clear_completion_cycle();
        self.clear_history_search();
        let start_byte = self.byte_index_for_cursor(start);
        let end_byte = self.byte_index_for_cursor(self.cursor);
        self.buffer.replace_range(start_byte..end_byte, "");
        self.cursor = start;
        self.history_cursor = None;
        Some(self.buffer.clone())
    }

    fn delete_next_word(&mut self) -> Option<String> {
        let end = self.next_word_boundary();
        if end == self.cursor {
            return Some(self.buffer.clone());
        }
        self.push_undo();
        self.clear_completion_cycle();
        self.clear_history_search();
        let start_byte = self.byte_index_for_cursor(self.cursor);
        let end_byte = self.byte_index_for_cursor(end);
        self.buffer.replace_range(start_byte..end_byte, "");
        self.history_cursor = None;
        Some(self.buffer.clone())
    }

    fn delete_before_cursor(&mut self) -> Option<String> {
        if self.cursor == 0 {
            return Some(self.buffer.clone());
        }
        self.push_undo();
        self.clear_completion_cycle();
        self.clear_history_search();
        let end = self.byte_index_for_cursor(self.cursor);
        self.buffer.replace_range(0..end, "");
        self.cursor = 0;
        self.history_cursor = None;
        Some(self.buffer.clone())
    }

    fn delete_after_cursor(&mut self) -> Option<String> {
        if self.cursor >= self.buffer.chars().count() {
            return Some(self.buffer.clone());
        }
        self.push_undo();
        self.clear_completion_cycle();
        self.clear_history_search();
        let start = self.byte_index_for_cursor(self.cursor);
        self.buffer.replace_range(start.., "");
        self.history_cursor = None;
        Some(self.buffer.clone())
    }

    fn undo(&mut self) -> Option<String> {
        let (buffer, cursor) = self.undo_stack.pop()?;
        self.redo_stack.push((self.buffer.clone(), self.cursor));
        self.buffer = buffer;
        self.cursor = cursor.min(self.buffer.chars().count());
        self.history_cursor = None;
        self.clear_completion_cycle();
        self.clear_history_search();
        Some(self.buffer.clone())
    }

    fn redo(&mut self) -> Option<String> {
        let (buffer, cursor) = self.redo_stack.pop()?;
        self.undo_stack.push((self.buffer.clone(), self.cursor));
        self.buffer = buffer;
        self.cursor = cursor.min(self.buffer.chars().count());
        self.history_cursor = None;
        self.clear_completion_cycle();
        self.clear_history_search();
        Some(self.buffer.clone())
    }

    fn push_undo(&mut self) {
        self.undo_stack.push((self.buffer.clone(), self.cursor));
        self.redo_stack.clear();
    }

    fn remove_char_at(&mut self, char_index: usize) {
        let start = self.byte_index_for_cursor(char_index);
        let end = self.byte_index_for_cursor(char_index + 1);
        self.buffer.replace_range(start..end, "");
    }

    fn previous_word_boundary(&self) -> usize {
        let chars = self.buffer.chars().collect::<Vec<_>>();
        let mut index = self.cursor.min(chars.len());
        while index > 0 && chars[index - 1].is_whitespace() {
            index -= 1;
        }
        while index > 0 && !chars[index - 1].is_whitespace() {
            index -= 1;
        }
        index
    }

    fn next_word_boundary(&self) -> usize {
        let chars = self.buffer.chars().collect::<Vec<_>>();
        let mut index = self.cursor.min(chars.len());
        while index < chars.len() && chars[index].is_whitespace() {
            index += 1;
        }
        while index < chars.len() && !chars[index].is_whitespace() {
            index += 1;
        }
        index
    }

    fn byte_index_for_cursor(&self, cursor: usize) -> usize {
        if cursor == 0 {
            return 0;
        }
        self.buffer
            .char_indices()
            .nth(cursor)
            .map(|(index, _)| index)
            .unwrap_or(self.buffer.len())
    }

    fn clear_completion_cycle(&mut self) {
        self.completion_cycle = None;
    }

    fn clear_history_search(&mut self) {
        self.history_search = None;
    }

    fn start_history_search_if_needed(&mut self) {
        if self.history_search.is_some() {
            return;
        }
        self.history_search = Some(WorkbenchHistorySearch {
            query: String::new(),
            matches: Vec::new(),
            index: 0,
            original_buffer: self.buffer.clone(),
            original_cursor: self.cursor,
        });
        self.refresh_history_search_matches();
    }

    fn refresh_history_search_matches(&mut self) {
        let Some(search) = &self.history_search else {
            return;
        };
        let query = search.query.to_ascii_lowercase();
        let matches = self
            .history
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, entry)| query.is_empty() || entry.to_ascii_lowercase().contains(&query))
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if let Some(search) = &mut self.history_search {
            search.matches = matches;
            search.index = search.index.min(search.matches.len().saturating_sub(1));
        }
        self.apply_history_search_selection();
    }

    fn cycle_history_search(&mut self, offset: usize) -> Option<String> {
        let Some(search) = &mut self.history_search else {
            return None;
        };
        if search.matches.is_empty() {
            self.apply_history_search_selection();
            return None;
        }
        search.index = (search.index + offset) % search.matches.len();
        self.apply_history_search_selection();
        Some(self.buffer.clone())
    }

    fn cycle_history_search_reverse(&mut self) -> Option<String> {
        let Some(search) = &mut self.history_search else {
            return None;
        };
        if search.matches.is_empty() {
            self.apply_history_search_selection();
            return None;
        }
        search.index = if search.index == 0 {
            search.matches.len().saturating_sub(1)
        } else {
            search.index - 1
        };
        self.apply_history_search_selection();
        Some(self.buffer.clone())
    }

    fn apply_history_search_selection(&mut self) {
        let Some(search) = &self.history_search else {
            return;
        };
        let Some(history_index) = search.matches.get(search.index).copied() else {
            self.buffer = search.original_buffer.clone();
            self.cursor = self.buffer.chars().count();
            return;
        };
        if let Some(selected) = self.history.get(history_index) {
            self.buffer = selected.clone();
            self.cursor = self.buffer.chars().count();
        }
        self.history_cursor = None;
        self.clear_completion_cycle();
    }

    fn current_completion_token(&self) -> Option<String> {
        let trimmed = self.buffer.trim_start();
        if !trimmed.starts_with('/') {
            return None;
        }
        Some(
            trimmed
                .split_whitespace()
                .next()
                .unwrap_or(trimmed)
                .to_owned(),
        )
    }

    fn replace_completion_token(&mut self, replacement: &str, append_space: bool) {
        let leading_len = self.buffer.len() - self.buffer.trim_start().len();
        let trimmed = &self.buffer[leading_len..];
        let token_len = trimmed
            .split_whitespace()
            .next()
            .map(str::len)
            .unwrap_or(trimmed.len());
        let end = leading_len + token_len;
        let suffix = if append_space { " " } else { "" };
        self.buffer
            .replace_range(leading_len..end, &format!("{replacement}{suffix}"));
        self.cursor = self.buffer[..leading_len].chars().count()
            + replacement.chars().count()
            + suffix.chars().count();
        self.history_cursor = None;
    }
}

#[cfg(test)]
mod tests {
    use super::{
        WorkbenchInputAction, WorkbenchInputState, WorkbenchTerminalInputEvent,
        WorkbenchTerminalInputOutcome, apply_workbench_terminal_input_event,
        parse_workbench_terminal_key_event,
    };
    use crossterm::event::{Event as CrosstermEvent, KeyCode, KeyEvent, KeyModifiers};

    #[test]
    fn terminal_key_events_map_to_workbench_input_events() {
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::CompletionPrevious
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::CompletionNext
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::CompletionPagePrevious
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::PageDown,
                KeyModifiers::NONE
            )),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::CompletionPageNext
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::MoveLeft
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::MoveRight
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::MoveStart
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::MoveEnd
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Backspace,
                KeyModifiers::NONE
            )),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::DeletePrevious
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::DeleteNext
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::Complete
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Char('p'),
                KeyModifiers::CONTROL
            )),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::HistoryPrevious
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Char('n'),
                KeyModifiers::CONTROL
            )),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::HistoryNext
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Char('z'),
                KeyModifiers::CONTROL
            )),
            Some(WorkbenchTerminalInputEvent::Action(
                WorkbenchInputAction::Undo
            ))
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Submit)
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)),
            Some(WorkbenchTerminalInputEvent::InsertNewline)
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)),
            Some(WorkbenchTerminalInputEvent::InsertNewline)
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Char('j'),
                KeyModifiers::CONTROL
            )),
            Some(WorkbenchTerminalInputEvent::InsertNewline)
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            Some(WorkbenchTerminalInputEvent::Escape)
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Char('c'),
                KeyModifiers::CONTROL
            )),
            Some(WorkbenchTerminalInputEvent::Interrupt)
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Char('d'),
                KeyModifiers::CONTROL
            )),
            Some(WorkbenchTerminalInputEvent::EndOfInput)
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Char('l'),
                KeyModifiers::CONTROL
            )),
            Some(WorkbenchTerminalInputEvent::ClearSession)
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Char('你'),
                KeyModifiers::NONE
            )),
            Some(WorkbenchTerminalInputEvent::InsertText("你".into()))
        );
    }

    #[test]
    fn terminal_key_events_ignore_system_shortcuts() {
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Char('s'),
                KeyModifiers::SHIFT | KeyModifiers::SUPER
            )),
            None
        );
        assert_eq!(
            parse_workbench_terminal_key_event(KeyEvent::new(
                KeyCode::Char('x'),
                KeyModifiers::META
            )),
            None
        );
    }

    #[test]
    fn terminal_input_reducer_edits_submits_clears_and_exits_without_line_mode() {
        let mut state = WorkbenchInputState::from_history(["/status".into()]);

        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::InsertText("/sta".into())
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(state.buffer(), "/sta");
        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::Action(WorkbenchInputAction::Complete)
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(state.buffer(), "/status ");
        assert_eq!(
            apply_workbench_terminal_input_event(&mut state, WorkbenchTerminalInputEvent::Submit),
            WorkbenchTerminalInputOutcome::Submit("/status".into())
        );
        assert_eq!(state.buffer(), "");
        assert_eq!(
            apply_workbench_terminal_input_event(&mut state, WorkbenchTerminalInputEvent::Submit),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::Interrupt
            ),
            WorkbenchTerminalInputOutcome::Exit
        );

        state.set_buffer("draft");
        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::Interrupt
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(state.buffer(), "");
    }

    #[test]
    fn terminal_input_reducer_preserves_explicit_multiline_messages() {
        let mut state = WorkbenchInputState::default();

        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::InsertText("first".into())
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::InsertNewline
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::InsertText("second".into())
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(state.buffer(), "first\nsecond");
        assert_eq!(
            apply_workbench_terminal_input_event(&mut state, WorkbenchTerminalInputEvent::Submit),
            WorkbenchTerminalInputOutcome::Submit("first\nsecond".into())
        );
    }

    #[test]
    fn terminal_input_reducer_drops_bare_mouse_fragments_from_buffer() {
        let mut state = WorkbenchInputState::default();

        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::InsertText("[<35;55;37M".into())
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(state.buffer(), "");

        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::InsertText("hello[<35;55;37m".into())
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(state.buffer(), "hello");
    }

    #[test]
    fn terminal_input_reducer_drops_double_open_mouse_fragments_from_buffer() {
        let mut state = WorkbenchInputState::default();

        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::InsertText("[[<35;55;37M".into())
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(state.buffer(), "");

        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::InsertText("hello[[<35;55;37m".into())
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(state.buffer(), "hello");
    }

    #[test]
    fn terminal_input_reducer_hides_partial_mouse_tail_from_buffer() {
        let mut state = WorkbenchInputState::default();

        assert_eq!(
            apply_workbench_terminal_input_event(
                &mut state,
                WorkbenchTerminalInputEvent::InsertText("hello[<35;55".into())
            ),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(state.buffer(), "hello");
    }

    #[test]
    fn terminal_paste_event_inserts_redacted_single_line_text() {
        let mut state = WorkbenchInputState::default();
        let event = super::parse_workbench_terminal_event(CrosstermEvent::Paste(
            "hello\napi_key=sk-secret-value\tworld".into(),
        ))
        .expect("paste event");

        assert_eq!(
            apply_workbench_terminal_input_event(&mut state, event),
            WorkbenchTerminalInputOutcome::Pending
        );
        assert_eq!(state.buffer(), "hello_[REDACTED_SECRET]_world");
        assert_eq!(
            apply_workbench_terminal_input_event(&mut state, WorkbenchTerminalInputEvent::Submit),
            WorkbenchTerminalInputOutcome::Submit("hello_[REDACTED_SECRET]_world".into())
        );
    }
}
