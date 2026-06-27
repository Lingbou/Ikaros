// SPDX-License-Identifier: GPL-3.0-only

use crate::text::{fit_terminal_text, terminal_display_width};
use crate::{
    WorkbenchInputState, WorkbenchTerminalInputOutcome, apply_workbench_terminal_input_event,
    fullscreen_terminal_event_input_available, parse_workbench_terminal_event, terminal_inline,
};
use anyhow::Result;
use crossterm::{
    cursor::{MoveToColumn, MoveUp},
    event, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode, size as terminal_size},
};
use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[derive(Clone)]
pub struct RunningTurnTerminal {
    inner: Arc<Mutex<RunningTurnTerminalState>>,
}

#[derive(Debug)]
struct RunningTurnTerminalState {
    rows: u16,
    cursor_row_from_top: u16,
    input: String,
    input_cursor_prefix: String,
    queued: usize,
}

impl RunningTurnTerminal {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(RunningTurnTerminalState {
                rows: 0,
                cursor_row_from_top: 0,
                input: String::new(),
                input_cursor_prefix: String::new(),
                queued: 0,
            })),
        }
    }

    fn render_input(&self, input: &WorkbenchInputState, queued: usize) -> Result<()> {
        let preview = running_input_preview(input.buffer());
        let cursor_prefix = running_input_cursor_prefix(input);
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("running turn terminal lock is poisoned"))?;
        inner.input = preview;
        inner.input_cursor_prefix = cursor_prefix;
        inner.queued = queued;
        let mut stdout = io::stdout();
        let had_rows = inner.rows > 0;
        clear_running_turn_composer_locked(&mut stdout, &mut inner)?;
        render_running_turn_composer_locked(&mut stdout, &mut inner, !had_rows)?;
        stdout.flush()?;
        Ok(())
    }

    pub fn print_output(&self, text: &str) -> Result<()> {
        if text.is_empty() {
            return Ok(());
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("running turn terminal lock is poisoned"))?;
        let mut stdout = io::stdout();
        clear_running_turn_composer_locked(&mut stdout, &mut inner)?;
        queue!(stdout, Print(normalize_raw_terminal_newlines(text)))?;
        if !text.ends_with(['\n', '\r']) {
            queue!(stdout, Print("\r\n"))?;
        }
        render_running_turn_composer_locked(&mut stdout, &mut inner, true)?;
        stdout.flush()?;
        Ok(())
    }

    fn clear(&self) -> Result<()> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|_| anyhow::anyhow!("running turn terminal lock is poisoned"))?;
        let mut stdout = io::stdout();
        clear_running_turn_composer_locked(&mut stdout, &mut inner)?;
        stdout.flush()?;
        Ok(())
    }
}

pub struct RunningTurnInputCapture {
    terminal: RunningTurnTerminal,
    stop: Arc<AtomicBool>,
    queued: Arc<Mutex<Vec<String>>>,
    raw_mode_enabled: bool,
    handle: Option<JoinHandle<()>>,
}

impl RunningTurnInputCapture {
    pub fn start() -> Option<Self> {
        if !fullscreen_terminal_event_input_available() || enable_raw_mode().is_err() {
            return None;
        }
        let terminal = RunningTurnTerminal::new();
        let stop = Arc::new(AtomicBool::new(false));
        let queued = Arc::new(Mutex::new(Vec::new()));
        let thread_terminal = terminal.clone();
        let thread_stop = Arc::clone(&stop);
        let thread_queued = Arc::clone(&queued);
        let handle = thread::spawn(move || {
            let mut input_state = WorkbenchInputState::default();
            let _ = thread_terminal.render_input(&input_state, 0);
            while !thread_stop.load(Ordering::Relaxed) {
                let Ok(ready) = event::poll(Duration::from_millis(50)) else {
                    continue;
                };
                if !ready {
                    continue;
                }
                let Ok(terminal_event) = event::read() else {
                    continue;
                };
                let Some(input_event) = parse_workbench_terminal_event(terminal_event) else {
                    continue;
                };
                match apply_workbench_terminal_input_event(&mut input_state, input_event) {
                    WorkbenchTerminalInputOutcome::Pending => {}
                    WorkbenchTerminalInputOutcome::Submit(input) => {
                        if let Ok(mut queued) = thread_queued.lock() {
                            queued.push(input);
                            let _ = thread_terminal.render_input(&input_state, queued.len());
                            continue;
                        }
                    }
                    WorkbenchTerminalInputOutcome::Exit => {
                        thread_stop.store(true, Ordering::Relaxed);
                    }
                }
                let queued_count = thread_queued.lock().map(|queued| queued.len()).unwrap_or(0);
                let _ = thread_terminal.render_input(&input_state, queued_count);
            }
        });
        Some(Self {
            terminal,
            stop,
            queued,
            raw_mode_enabled: true,
            handle: Some(handle),
        })
    }

    pub fn terminal(&self) -> RunningTurnTerminal {
        self.terminal.clone()
    }

    pub fn finish(mut self) -> Vec<String> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let _ = self.terminal.clear();
        if self.raw_mode_enabled {
            let _ = disable_raw_mode();
            self.raw_mode_enabled = false;
        }
        self.queued
            .lock()
            .map(|queued| queued.clone())
            .unwrap_or_default()
    }
}

impl Drop for RunningTurnInputCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        let _ = self.terminal.clear();
        if self.raw_mode_enabled {
            let _ = disable_raw_mode();
            self.raw_mode_enabled = false;
        }
    }
}

fn running_input_preview(input: &str) -> String {
    terminal_inline(input).replace('\n', " * ")
}

fn running_input_cursor_prefix(input: &WorkbenchInputState) -> String {
    let before_cursor = input
        .buffer()
        .chars()
        .take(input.cursor())
        .collect::<String>();
    running_input_preview(&before_cursor)
}

fn clear_running_turn_composer_locked(
    stdout: &mut impl Write,
    state: &mut RunningTurnTerminalState,
) -> Result<()> {
    if state.rows == 0 {
        return Ok(());
    }
    if state.cursor_row_from_top > 0 {
        queue!(stdout, MoveUp(state.cursor_row_from_top))?;
    }
    queue!(stdout, MoveToColumn(0))?;
    for index in 0..state.rows {
        if index > 0 {
            queue!(stdout, Print("\r\n"))?;
        }
        queue!(stdout, Clear(ClearType::CurrentLine))?;
    }
    if state.rows > 1 {
        queue!(stdout, MoveUp(state.rows - 1))?;
    }
    queue!(stdout, MoveToColumn(0))?;
    state.rows = 0;
    state.cursor_row_from_top = 0;
    Ok(())
}

fn render_running_turn_composer_locked(
    stdout: &mut impl Write,
    state: &mut RunningTurnTerminalState,
    leading_gap: bool,
) -> Result<()> {
    let width = terminal_width();
    if leading_gap {
        queue!(stdout, Print("\r\n"))?;
    }
    let cursor = queue_running_turn_composer_block(stdout, state, width)?;
    state.rows = 3;
    state.cursor_row_from_top = cursor.row;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RunningTurnComposerCursor {
    row: u16,
    column: u16,
}

fn queue_running_turn_composer_block(
    stdout: &mut impl Write,
    state: &RunningTurnTerminalState,
    width: usize,
) -> Result<RunningTurnComposerCursor> {
    let cursor_column = queue_running_turn_composer_prompt_line(stdout, state, width)?;
    queue!(stdout, Print("\r\n"))?;
    queue_running_turn_composer_padding_line(stdout)?;
    queue!(stdout, MoveUp(1), MoveToColumn(cursor_column))?;
    Ok(RunningTurnComposerCursor {
        row: 1,
        column: cursor_column,
    })
}

fn queue_running_turn_composer_padding_line(stdout: &mut impl Write) -> Result<()> {
    queue!(
        stdout,
        SetBackgroundColor(Color::DarkGrey),
        Clear(ClearType::UntilNewLine),
        ResetColor
    )?;
    Ok(())
}

fn queue_running_turn_composer_prompt_line(
    stdout: &mut impl Write,
    state: &RunningTurnTerminalState,
    width: usize,
) -> Result<u16> {
    let width = width.max(4);
    let input_is_empty = state.input.is_empty();
    let input = if input_is_empty {
        "Ask Ikaros while I work"
    } else {
        state.input.as_str()
    };
    let suffix = (state.queued > 0).then(|| format!("queued {}", state.queued));
    let suffix_width = suffix
        .as_deref()
        .map(terminal_display_width)
        .unwrap_or_default();
    let gap_width = usize::from(suffix.is_some());
    let input_width = width.saturating_sub(2 + suffix_width + gap_width).max(1);
    let input = fit_terminal_text(input, input_width);
    let cursor_source = if input_is_empty {
        ""
    } else {
        state.input_cursor_prefix.as_str()
    };
    let cursor_source = fit_terminal_text(cursor_source, input_width);
    let cursor_column = (2 + terminal_display_width(&cursor_source)).min(width.saturating_sub(1));
    queue_running_turn_composer_padding_line(stdout)?;
    queue!(stdout, Print("\r\n"))?;
    let mut segments = vec![
        (ikaros_accent_color(), "* ".to_owned()),
        (
            if input_is_empty {
                Color::Grey
            } else {
                Color::White
            },
            input,
        ),
    ];
    if let Some(suffix) = suffix {
        segments.push((Color::Grey, format!(" {suffix}")));
    }
    queue_running_turn_composer_text_line(
        stdout,
        segments.iter().map(|(color, text)| (*color, text.as_str())),
    )?;
    queue!(stdout, MoveToColumn(cursor_column as u16))?;
    Ok(cursor_column as u16)
}

fn queue_running_turn_composer_text_line<'a>(
    stdout: &mut impl Write,
    segments: impl IntoIterator<Item = (Color, &'a str)>,
) -> Result<()> {
    queue!(
        stdout,
        SetBackgroundColor(Color::DarkGrey),
        Clear(ClearType::UntilNewLine),
        MoveToColumn(0)
    )?;
    for (color, text) in segments {
        queue!(stdout, SetForegroundColor(color), Print(text))?;
    }
    queue!(stdout, ResetColor)?;
    Ok(())
}

pub fn normalize_raw_terminal_newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous_was_cr = false;
    for ch in text.chars() {
        if ch == '\n' {
            if !previous_was_cr {
                out.push('\r');
            }
            out.push('\n');
            previous_was_cr = false;
        } else {
            previous_was_cr = ch == '\r';
            out.push(ch);
        }
    }
    out
}

fn terminal_width() -> usize {
    terminal_size()
        .map(|(width, _)| usize::from(width).max(20))
        .unwrap_or(80)
}

fn ikaros_accent_color() -> Color {
    Color::Rgb {
        r: 255,
        g: 154,
        b: 252,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::strip_ansi_sequences;

    #[test]
    fn raw_terminal_newline_normalization_returns_to_line_start() {
        assert_eq!(
            normalize_raw_terminal_newlines("first\n\nsecond\r\nthird"),
            "first\r\n\r\nsecond\r\nthird"
        );
    }

    #[test]
    fn running_turn_composer_keeps_cursor_in_input_box() {
        let state = RunningTurnTerminalState {
            rows: 0,
            cursor_row_from_top: 0,
            input: String::new(),
            input_cursor_prefix: String::new(),
            queued: 0,
        };
        let mut output = Vec::new();

        let cursor =
            queue_running_turn_composer_block(&mut output, &state, 32).expect("composer block");
        let output = String::from_utf8(output).expect("composer output is utf8");
        let visible = strip_ansi_sequences(&output);
        let rows = visible.split("\r\n").collect::<Vec<_>>();

        assert_eq!(cursor, RunningTurnComposerCursor { row: 1, column: 2 });
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], "");
        assert!(rows[1].starts_with("* Ask Ikaros while I work"));
        assert!(!rows[1].ends_with(' '));
        assert_eq!(rows[2], "");
        assert!(output.contains("\x1b[K"));
        assert!(output.contains("\x1b[K\x1b[1G"));
        assert!(output.contains("\x1b[K\x1b[0m"));
        assert!(!output.contains("Enter queues next"));
        assert!(output.ends_with("\x1b[1A\x1b[3G"));
    }

    #[test]
    fn running_turn_composer_keeps_queued_hint_inside_input_box() {
        let state = RunningTurnTerminalState {
            rows: 0,
            cursor_row_from_top: 0,
            input: "next".to_owned(),
            input_cursor_prefix: "next".to_owned(),
            queued: 2,
        };
        let mut output = Vec::new();

        let cursor =
            queue_running_turn_composer_block(&mut output, &state, 24).expect("composer block");
        let output = String::from_utf8(output).expect("composer output is utf8");
        let visible = strip_ansi_sequences(&output);
        let rows = visible.split("\r\n").collect::<Vec<_>>();

        assert_eq!(cursor, RunningTurnComposerCursor { row: 1, column: 6 });
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], "");
        assert!(rows[1].starts_with("* next"));
        assert!(rows[1].contains("queued 2"));
        assert!(!rows[1].ends_with(' '));
        assert_eq!(rows[2], "");
        assert!(output.contains("\x1b[K"));
        assert!(output.contains("\x1b[K\x1b[1G"));
        assert!(output.contains("\x1b[K\x1b[0m"));
        assert!(output.ends_with("\x1b[1A\x1b[7G"));
    }

    #[test]
    fn running_turn_composer_adds_gap_before_input_block() {
        let mut state = RunningTurnTerminalState {
            rows: 0,
            cursor_row_from_top: 0,
            input: String::new(),
            input_cursor_prefix: String::new(),
            queued: 0,
        };
        let mut output = Vec::new();

        render_running_turn_composer_locked(&mut output, &mut state, true)
            .expect("render composer");
        let output = String::from_utf8(output).expect("composer output is utf8");

        assert!(output.starts_with("\r\n"));
        assert_eq!(state.rows, 3);
        assert_eq!(state.cursor_row_from_top, 1);
    }
}
