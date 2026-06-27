// SPDX-License-Identifier: GPL-3.0-only

use std::mem;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TerminalMode {
    #[default]
    MainScreen,
    FullscreenReserved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerState {
    terminal_mode: TerminalMode,
    input: String,
}

impl Default for ComposerState {
    fn default() -> Self {
        Self {
            terminal_mode: TerminalMode::MainScreen,
            input: String::new(),
        }
    }
}

impl ComposerState {
    pub fn new(terminal_mode: TerminalMode) -> Self {
        Self {
            terminal_mode,
            input: String::new(),
        }
    }

    pub fn with_input(terminal_mode: TerminalMode, input: impl Into<String>) -> Self {
        Self {
            terminal_mode,
            input: input.into(),
        }
    }

    pub fn terminal_mode(&self) -> TerminalMode {
        self.terminal_mode
    }

    pub fn input(&self) -> &str {
        &self.input
    }

    pub fn is_empty(&self) -> bool {
        self.input.is_empty()
    }

    pub fn apply(&mut self, action: ComposerAction) -> ComposerOutcome {
        match action {
            ComposerAction::InsertChar(ch) => {
                self.input.push(ch);
                ComposerOutcome::Updated
            }
            ComposerAction::InsertText(text) => {
                self.input.push_str(&text);
                ComposerOutcome::Updated
            }
            ComposerAction::Backspace => {
                let _ = self.input.pop();
                ComposerOutcome::Updated
            }
            ComposerAction::Clear => {
                self.input.clear();
                ComposerOutcome::Updated
            }
            ComposerAction::Enter => self.submit(),
            ComposerAction::ShiftEnter | ComposerAction::AltEnter => {
                self.input.push('\n');
                ComposerOutcome::NewlineInserted
            }
        }
    }

    fn submit(&mut self) -> ComposerOutcome {
        if self.input.trim().is_empty() {
            return ComposerOutcome::Ignored;
        }

        ComposerOutcome::Submitted(mem::take(&mut self.input))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerAction {
    InsertChar(char),
    InsertText(String),
    Backspace,
    Clear,
    Enter,
    ShiftEnter,
    AltEnter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerOutcome {
    Updated,
    NewlineInserted,
    Submitted(String),
    Ignored,
}
