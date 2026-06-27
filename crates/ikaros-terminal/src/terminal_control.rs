// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use crossterm::{
    Command,
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, KeyboardEnhancementFlags,
        PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::{
    fmt,
    io::{self, IsTerminal},
};

pub fn fullscreen_terminal_event_input_available() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub fn disable_mouse_tracking_best_effort() {
    if !io::stdout().is_terminal() {
        return;
    }
    let _ = crossterm::execute!(io::stdout(), DisableMouseTrackingModes, DisableMouseCapture);
}

pub struct WorkbenchTerminalInputSessionGuard {
    keyboard_enhancements_pushed: bool,
}

impl WorkbenchTerminalInputSessionGuard {
    pub fn enable() -> Result<Self> {
        crossterm::execute!(
            io::stdout(),
            DisableMouseTrackingModes,
            DisableMouseCapture,
            EnableBracketedPaste,
        )?;
        Ok(Self {
            keyboard_enhancements_pushed: push_keyboard_enhancements_best_effort(),
        })
    }
}

impl Drop for WorkbenchTerminalInputSessionGuard {
    fn drop(&mut self) {
        if self.keyboard_enhancements_pushed {
            pop_keyboard_enhancements_best_effort();
        }
        let _ = crossterm::execute!(
            io::stdout(),
            DisableBracketedPaste,
            DisableMouseTrackingModes,
            DisableMouseCapture
        );
    }
}

pub struct FullscreenRawModeGuard {
    terminal_modes_owned: bool,
    keyboard_enhancements_pushed: bool,
}

impl FullscreenRawModeGuard {
    pub fn enable_for_line_input(terminal_modes_owned: bool) -> Result<Self> {
        enable_raw_mode()?;
        if terminal_modes_owned
            && let Err(error) = crossterm::execute!(
                io::stdout(),
                DisableMouseTrackingModes,
                DisableMouseCapture,
                EnableBracketedPaste
            )
        {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        let keyboard_enhancements_pushed = if terminal_modes_owned {
            push_keyboard_enhancements_best_effort()
        } else {
            false
        };
        Ok(Self {
            terminal_modes_owned,
            keyboard_enhancements_pushed,
        })
    }

    pub fn owns_terminal_modes(&self) -> bool {
        self.terminal_modes_owned
    }
}

impl Drop for FullscreenRawModeGuard {
    fn drop(&mut self) {
        if self.keyboard_enhancements_pushed {
            pop_keyboard_enhancements_best_effort();
        }
        if self.terminal_modes_owned {
            let _ = crossterm::execute!(
                io::stdout(),
                DisableBracketedPaste,
                DisableMouseTrackingModes,
                DisableMouseCapture
            );
        }
        let _ = disable_raw_mode();
    }
}

fn push_keyboard_enhancements_best_effort() -> bool {
    if !io::stdout().is_terminal() {
        return false;
    }
    crossterm::execute!(
        io::stdout(),
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
                | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES,
        )
    )
    .is_ok()
}

fn pop_keyboard_enhancements_best_effort() {
    let _ = crossterm::execute!(io::stdout(), PopKeyboardEnhancementFlags);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DisableMouseTrackingModes;

impl Command for DisableMouseTrackingModes {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1007l")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(io::Error::other(
            "DisableMouseTrackingModes must be executed as ANSI",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SetScrollRegion(pub std::ops::Range<u16>);

impl Command for SetScrollRegion {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[{};{}r", self.0.start, self.0.end)
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(io::Error::other("SetScrollRegion must be executed as ANSI"))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetScrollRegion;

impl Command for ResetScrollRegion {
    fn write_ansi(&self, f: &mut impl fmt::Write) -> fmt::Result {
        write!(f, "\x1b[r")
    }

    #[cfg(windows)]
    fn execute_winapi(&self) -> io::Result<()> {
        Err(io::Error::other(
            "ResetScrollRegion must be executed as ANSI",
        ))
    }

    #[cfg(windows)]
    fn is_ansi_code_supported(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::FullscreenRawModeGuard;

    #[test]
    fn raw_mode_guard_can_borrow_session_terminal_modes() {
        let guard = FullscreenRawModeGuard {
            terminal_modes_owned: false,
            keyboard_enhancements_pushed: false,
        };

        assert!(!guard.owns_terminal_modes());
    }
}
