// SPDX-License-Identifier: GPL-3.0-only

use super::workbench::{
    self, WorkbenchInputAction, WorkbenchInputEvent, WorkbenchInputState,
    WorkbenchTerminalInputOutcome, apply_workbench_terminal_input_event,
    format_workbench_input_state, parse_workbench_input_event, parse_workbench_terminal_event,
    terminal_inline,
};
use anyhow::Result;
use crossterm::{
    Command,
    cursor::{MoveTo, MoveToColumn, MoveUp},
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
        Event as CrosstermEvent, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode, size as terminal_size},
};
use ikaros_tui::{SlashCommandPopup, SlashCommandSpec};
use std::{
    fmt,
    io::{self, IsTerminal, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) const BRACKETED_PASTE_START: &str = "\u{1b}[200~";
const BRACKETED_PASTE_END: &str = "\u{1b}[201~";
const INLINE_COMPOSER_HINT_LIMIT: usize = 5;

fn ikaros_accent_color() -> Color {
    Color::Rgb {
        r: 255,
        g: 154,
        b: 252,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct WorkbenchLineInputUi {
    model_label: String,
    workspace_label: String,
    show_intro: bool,
}

impl WorkbenchLineInputUi {
    pub(super) fn new(
        model_label: impl Into<String>,
        workspace_label: impl Into<String>,
        show_intro: bool,
    ) -> Self {
        Self {
            model_label: terminal_inline(&model_label.into()),
            workspace_label: terminal_inline(&workspace_label.into()),
            show_intro,
        }
    }
}

#[derive(Clone)]
pub(super) struct RunningTurnTerminal {
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

    pub(super) fn print_output(&self, text: &str) -> Result<()> {
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

pub(super) struct RunningTurnInputCapture {
    terminal: RunningTurnTerminal,
    stop: Arc<AtomicBool>,
    queued: Arc<Mutex<Vec<String>>>,
    raw_mode_enabled: bool,
    handle: Option<JoinHandle<()>>,
}

impl RunningTurnInputCapture {
    pub(super) fn start() -> Option<Self> {
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

    pub(super) fn terminal(&self) -> RunningTurnTerminal {
        self.terminal.clone()
    }

    pub(super) fn finish(mut self) -> Vec<String> {
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
    terminal_inline(input).replace('\n', " ↵ ")
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
    queue_running_turn_composer_padding_line(stdout, width)?;
    queue!(stdout, MoveUp(1), MoveToColumn(cursor_column))?;
    Ok(RunningTurnComposerCursor {
        row: 1,
        column: cursor_column,
    })
}

fn queue_running_turn_composer_padding_line(stdout: &mut impl Write, _width: usize) -> Result<()> {
    queue_codex_input_cell_padding_line(stdout)?;
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
    queue_running_turn_composer_padding_line(stdout, width)?;
    queue!(stdout, Print("\r\n"))?;
    let mut segments = vec![
        (ikaros_accent_color(), "› ".to_owned()),
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
    queue_codex_input_cell_text_line(
        stdout,
        segments.iter().map(|(color, text)| (*color, text.as_str())),
    )?;
    queue!(stdout, MoveToColumn(cursor_column as u16))?;
    Ok(cursor_column as u16)
}

pub(super) fn normalize_raw_terminal_newlines(text: &str) -> String {
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

pub(super) fn fullscreen_terminal_event_input_available() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

pub(super) fn read_workbench_terminal_line_input(
    input_state: &mut WorkbenchInputState,
    fallback_line: &mut String,
    ui: Option<&WorkbenchLineInputUi>,
    terminal_modes_already_enabled: bool,
    render_state: &mut WorkbenchLineEditorRenderState,
) -> Result<Option<String>> {
    if !fullscreen_terminal_event_input_available() {
        return read_workbench_stdio_line_input(fallback_line);
    }
    disable_mouse_tracking_best_effort();
    if let Some(ui) = ui.filter(|ui| ui.show_intro) {
        render_workbench_inline_intro(ui)?;
    }
    let _raw_mode =
        match FullscreenRawModeGuard::enable_for_line_input(!terminal_modes_already_enabled) {
            Ok(guard) => guard,
            Err(error) => {
                if ui.is_none() {
                    println!(
                        "workbench_input_mode: line fallback=raw_unavailable reason={}",
                        terminal_inline(&error.to_string())
                    );
                }
                return read_workbench_stdio_line_input(fallback_line);
            }
        };
    input_state.set_buffer("");
    render_workbench_line_editor(input_state, ui, render_state)?;
    loop {
        let terminal_event = event::read()?;
        if matches!(terminal_event, CrosstermEvent::Resize(_, _)) {
            render_workbench_line_editor(input_state, ui, render_state)?;
            continue;
        }
        let Some(input_event) = parse_workbench_terminal_event(terminal_event) else {
            continue;
        };
        match apply_workbench_terminal_input_event(input_state, input_event) {
            WorkbenchTerminalInputOutcome::Pending => {
                render_workbench_line_editor(input_state, ui, render_state)?;
            }
            WorkbenchTerminalInputOutcome::Submit(input) => {
                render_state.clear_rows_before_history_insert()?;
                if ui.is_some() {
                    insert_submitted_user_message(&input)?;
                } else {
                    println!();
                }
                return Ok(Some(input));
            }
            WorkbenchTerminalInputOutcome::Exit => {
                render_state.clear_rows_before_history_insert()?;
                if ui.is_none() {
                    println!();
                }
                return Ok(None);
            }
        }
    }
}

fn read_workbench_stdio_line_input(fallback_line: &mut String) -> Result<Option<String>> {
    disable_mouse_tracking_best_effort();
    print!("› ");
    io::stdout().flush()?;
    fallback_line.clear();
    if io::stdin().read_line(fallback_line)? == 0 {
        return Ok(None);
    }
    Ok(Some(
        fallback_line.trim_end_matches(['\n', '\r']).to_owned(),
    ))
}

fn disable_mouse_tracking_best_effort() {
    if !io::stdout().is_terminal() {
        return;
    }
    let _ = crossterm::execute!(io::stdout(), DisableMouseTrackingModes, DisableMouseCapture);
}

fn render_workbench_line_editor(
    input_state: &WorkbenchInputState,
    ui: Option<&WorkbenchLineInputUi>,
    state: &mut WorkbenchLineEditorRenderState,
) -> Result<()> {
    if let Some(ui) = ui {
        return render_workbench_inline_composer(input_state, ui, state);
    }
    let completions = input_state.completion_candidates();
    let completion_hint = if completions.is_empty() {
        String::new()
    } else {
        format!("  [tab: {}]", terminal_inline(&completions.join(", ")))
    };
    let history_search_hint = if input_state.history_search_active() {
        let candidates = input_state.history_search_candidates(3);
        let candidates = if candidates.is_empty() {
            "none".into()
        } else {
            terminal_inline(&candidates.join(" | "))
        };
        format!(
            "  [ctrl-r: {} matches={}]",
            terminal_inline(&input_state.history_search_summary()),
            candidates
        )
    } else {
        String::new()
    };
    let dirty_marker = if input_state.buffer().contains('\n') {
        " [multi]"
    } else if input_state.history_search_active() {
        " [search]"
    } else {
        ""
    };
    print!(
        "\r\x1b[2K›{} {}{}{}",
        dirty_marker,
        input_state.cursor_view(),
        completion_hint,
        history_search_hint
    );
    io::stdout().flush()?;
    state.rows = 1;
    Ok(())
}

fn render_workbench_inline_intro(ui: &WorkbenchLineInputUi) -> Result<()> {
    let width = terminal_width();
    let mut stdout = io::stdout();
    queue_workbench_inline_intro(&mut stdout, ui, width)?;
    stdout.flush()?;
    Ok(())
}

fn queue_workbench_inline_intro(
    stdout: &mut impl Write,
    ui: &WorkbenchLineInputUi,
    width: usize,
) -> Result<()> {
    let card_width = codex_intro_card_width(ui, width);
    let inner_width = card_width.saturating_sub(4);
    let label_width = "directory:".len();
    let model_label = format!("{:<label_width$}", "model:");
    let directory_label = format!("{:<label_width$}", "directory:");
    let model_line = format!("{model_label} {}   /model to change", ui.model_label);
    let directory_value_width = inner_width.saturating_sub(directory_label.len() + 1);
    let directory_line = format!(
        "{directory_label} {}",
        compact_path_label(&ui.workspace_label, directory_value_width)
    );

    queue!(
        stdout,
        SetForegroundColor(Color::Green),
        SetAttribute(Attribute::Bold),
        Print("ikaros"),
        SetAttribute(Attribute::Reset),
        ResetColor,
        Print("\r\n\r\n"),
        SetForegroundColor(Color::DarkGrey),
        Print(codex_intro_border('╭', '╮', card_width)),
        ResetColor,
        Print("\r\n")
    )?;
    queue_codex_intro_box_line(
        stdout,
        &format!(">_ Ikaros (v{})", env!("CARGO_PKG_VERSION")),
        card_width,
    )?;
    queue_codex_intro_box_line(stdout, "", card_width)?;
    queue_codex_intro_box_line(stdout, &model_line, card_width)?;
    queue_codex_intro_box_line(stdout, &directory_line, card_width)?;
    queue!(
        stdout,
        SetForegroundColor(Color::DarkGrey),
        Print(codex_intro_border('╰', '╯', card_width)),
        ResetColor,
        Print("\r\n\r\n")
    )?;
    Ok(())
}

fn queue_codex_intro_box_line(stdout: &mut impl Write, content: &str, width: usize) -> Result<()> {
    let inner_width = width.saturating_sub(4);
    let content = fit_terminal_text(content, inner_width);
    let padded = pad_terminal_text(&content, inner_width);
    queue!(
        stdout,
        SetForegroundColor(Color::DarkGrey),
        Print("│"),
        ResetColor,
        Print(" "),
        Print(padded),
        Print(" "),
        SetForegroundColor(Color::DarkGrey),
        Print("│"),
        ResetColor,
        Print("\r\n")
    )?;
    Ok(())
}

fn codex_intro_card_width(ui: &WorkbenchLineInputUi, terminal_width: usize) -> usize {
    let label_width = "directory:".len();
    let title_width =
        terminal_display_width(&format!(">_ Ikaros (v{})", env!("CARGO_PKG_VERSION")));
    let model_width =
        label_width + 1 + terminal_display_width(&ui.model_label) + "   /model to change".len();
    let directory_width = label_width + 1 + terminal_display_width(&ui.workspace_label);
    let content_width = title_width.max(model_width).max(directory_width);
    let target = content_width.saturating_add(4).clamp(42, 56);
    target.min(terminal_width.max(20))
}

fn codex_intro_border(left: char, right: char, width: usize) -> String {
    let inner = width.saturating_sub(2);
    format!("{left}{}{right}", "─".repeat(inner))
}

#[cfg(test)]
fn codex_intro_box_line(content: &str, width: usize) -> String {
    let inner_width = width.saturating_sub(4);
    let content = fit_terminal_text(content, inner_width);
    format!("│ {} │", pad_terminal_text(&content, inner_width))
}

fn render_workbench_inline_composer(
    input_state: &WorkbenchInputState,
    ui: &WorkbenchLineInputUi,
    state: &mut WorkbenchLineEditorRenderState,
) -> Result<()> {
    let (width, height) = terminal_dimensions();
    let lines = inline_composer_lines(input_state, ui, width);
    let cursor = inline_composer_cursor(input_state, width);
    let previous_rows = state.rows;
    let repaint_rows = previous_rows.max(lines.len() as u16);
    let mut stdout = io::stdout();
    let current_cursor_y = crossterm::cursor::position().ok().map(|(_, y)| y);
    let mut composer_top = inline_composer_repaint_top(
        state.composer_top,
        previous_rows,
        state.cursor_row_from_top,
        state.viewport_anchored,
        current_cursor_y,
    );
    if previous_rows == 0 && !state.viewport_anchored {
        let cursor_y = current_cursor_y.unwrap_or(0);
        let spacer_rows = inline_viewport_spacer_rows(cursor_y, height, lines.len() as u16);
        for _ in 0..spacer_rows {
            queue!(stdout, Print("\r\n"))?;
        }
        composer_top = Some(cursor_y.saturating_add(spacer_rows));
        state.viewport_anchored = true;
    } else if previous_rows > 0 {
        if let Some(top) = composer_top {
            queue!(stdout, MoveTo(0, top))?;
        } else if previous_rows > 1 {
            queue!(stdout, MoveUp(previous_rows - 1))?;
        }
    }
    for index in 0..usize::from(repaint_rows) {
        if index > 0 {
            queue!(stdout, Print("\r\n"))?;
        }
        queue!(stdout, MoveToColumn(0))?;
        if let Some(line) = lines.get(index) {
            queue_inline_composer_line(&mut stdout, line, width)?;
        } else {
            queue!(stdout, Clear(ClearType::CurrentLine))?;
        }
    }
    queue_inline_composer_cursor(&mut stdout, composer_top, cursor, repaint_rows)?;
    stdout.flush()?;
    state.rows = lines.len() as u16;
    state.cursor_row_from_top = cursor.row;
    state.composer_top = composer_top;
    Ok(())
}

fn inline_composer_repaint_top(
    anchored_top: Option<u16>,
    previous_rows: u16,
    cursor_row_from_top: u16,
    viewport_anchored: bool,
    current_cursor_y: Option<u16>,
) -> Option<u16> {
    if previous_rows > 0 {
        return anchored_top
            .or_else(|| current_cursor_y.map(|y| y.saturating_sub(cursor_row_from_top)));
    }
    if viewport_anchored {
        anchored_top
    } else {
        current_cursor_y
    }
}

fn queue_inline_composer_cursor(
    stdout: &mut impl Write,
    composer_top: Option<u16>,
    cursor: InlineComposerCursor,
    repaint_rows: u16,
) -> Result<()> {
    if let Some(top) = composer_top {
        queue!(
            stdout,
            MoveTo(cursor.column, top.saturating_add(cursor.row))
        )?;
        return Ok(());
    }
    let current_row = repaint_rows.saturating_sub(1);
    if current_row > cursor.row {
        queue!(stdout, MoveUp(current_row - cursor.row))?;
    }
    queue!(stdout, MoveToColumn(cursor.column))?;
    Ok(())
}

fn queue_inline_composer_line(
    stdout: &mut impl Write,
    line: &InlineComposerLine,
    width: usize,
) -> Result<()> {
    queue!(stdout, Clear(ClearType::CurrentLine))?;
    match line.style {
        InlineComposerLineStyle::Prompt => {
            if line.text.is_empty() {
                queue_codex_input_cell_padding_line(stdout)?;
            } else {
                let text = fit_terminal_text(&line.text, width);
                queue_codex_input_cell_text_line(stdout, [(Color::White, text.as_str())])?;
            }
        }
        InlineComposerLineStyle::Placeholder => {
            let rest = line.text.strip_prefix("› ").unwrap_or(&line.text);
            let rest = fit_terminal_text(rest, width.saturating_sub(2));
            queue_codex_input_cell_text_line(
                stdout,
                [(Color::White, "› "), (Color::Grey, rest.as_str())],
            )?;
        }
        InlineComposerLineStyle::Hint => {
            queue!(
                stdout,
                SetForegroundColor(Color::DarkGreen),
                Print(fit_terminal_text(&line.text, width)),
                ResetColor,
                Clear(ClearType::UntilNewLine)
            )?;
        }
        InlineComposerLineStyle::SelectedHint => {
            queue!(
                stdout,
                SetForegroundColor(Color::Cyan),
                Print(fit_terminal_text(&line.text, width)),
                ResetColor,
                Clear(ClearType::UntilNewLine)
            )?;
        }
        InlineComposerLineStyle::Footer => {
            queue_inline_footer(stdout, &line.text, width)?;
        }
    }
    Ok(())
}

fn queue_inline_footer(stdout: &mut impl Write, text: &str, width: usize) -> Result<()> {
    let text = fit_terminal_text(text, width);
    let Some((model_part, workspace_part)) = text.split_once(" · ") else {
        queue!(
            stdout,
            SetForegroundColor(Color::DarkYellow),
            Print(fit_terminal_text(&text, width)),
            ResetColor,
            Clear(ClearType::UntilNewLine)
        )?;
        return Ok(());
    };
    let leading = model_part
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .collect::<String>();
    let model = model_part.trim_start();
    queue!(
        stdout,
        SetForegroundColor(Color::DarkGrey),
        Print(leading),
        SetForegroundColor(Color::DarkYellow),
        Print(model),
        SetForegroundColor(Color::DarkGrey),
        Print(" · "),
        SetForegroundColor(Color::DarkGreen),
        Print(workspace_part),
        ResetColor,
        Clear(ClearType::UntilNewLine)
    )?;
    Ok(())
}

fn queue_codex_input_cell_padding_line(stdout: &mut impl Write) -> Result<()> {
    queue!(
        stdout,
        SetBackgroundColor(Color::DarkGrey),
        Clear(ClearType::UntilNewLine),
        ResetColor
    )?;
    Ok(())
}

fn queue_codex_input_cell_text_line<'a>(
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

fn insert_submitted_user_message(input: &str) -> Result<()> {
    if io::stdout().is_terminal()
        && let Ok((_, composer_top)) = crossterm::cursor::position()
        && composer_top >= 2
    {
        let width = terminal_width();
        let mut stdout = io::stdout();
        queue_submitted_user_message_history_insert(&mut stdout, input, width, composer_top)?;
        stdout.flush()?;
        return Ok(());
    }
    print_submitted_user_message_at_cursor(input)
}

fn print_submitted_user_message_at_cursor(input: &str) -> Result<()> {
    let width = terminal_width();
    let mut stdout = io::stdout();
    for (index, line) in submitted_user_message_cell_lines(input, width)
        .iter()
        .enumerate()
    {
        if index > 0 {
            queue!(stdout, Print("\r\n"))?;
        }
        queue_submitted_user_message_cell_line(&mut stdout, line, width)?;
    }
    queue!(stdout, Print("\r\n"))?;
    stdout.flush()?;
    Ok(())
}

fn queue_submitted_user_message_history_insert(
    stdout: &mut impl Write,
    input: &str,
    width: usize,
    composer_top: u16,
) -> Result<()> {
    let rows = submitted_user_message_cell_lines(input, width);
    queue!(stdout, SetScrollRegion(1..composer_top))?;
    queue!(stdout, MoveTo(0, composer_top.saturating_sub(1)))?;
    for line in &rows {
        queue!(stdout, Print("\r\n"))?;
        queue_submitted_user_message_cell_line(stdout, line, width)?;
    }
    queue!(stdout, ResetScrollRegion, MoveTo(0, composer_top))?;
    Ok(())
}

fn queue_submitted_user_message_cell_line(
    stdout: &mut impl Write,
    line: &SubmittedUserMessageCellLine,
    width: usize,
) -> Result<()> {
    match line.kind {
        SubmittedUserMessageCellLineKind::Spacer => {
            queue_codex_input_cell_padding_line(stdout)?;
        }
        SubmittedUserMessageCellLineKind::First => {
            let text = fit_terminal_text(&line.text, width.saturating_sub(2));
            queue_codex_input_cell_text_line(
                stdout,
                [(Color::Cyan, "› "), (Color::White, text.as_str())],
            )?;
        }
        SubmittedUserMessageCellLineKind::Continuation => {
            let text = fit_terminal_text(&line.text, width.saturating_sub(2));
            queue_codex_input_cell_text_line(
                stdout,
                [(Color::White, "  "), (Color::White, text.as_str())],
            )?;
        }
    }
    Ok(())
}

pub(super) fn print_inline_turn_separator() {
    print!("{}", inline_turn_separator_text(terminal_width()));
}

pub(super) fn print_inline_turn_worked_separator(elapsed_ms: u128) -> Result<()> {
    let width = terminal_width();
    let text = inline_turn_worked_separator_text(width, elapsed_ms);
    if !io::stdout().is_terminal() {
        print!("{text}");
        return Ok(());
    }
    let mut stdout = io::stdout();
    let label = inline_turn_worked_label(elapsed_ms);
    let line = inline_labeled_separator(width, &label);
    let prefix = match crossterm::cursor::position() {
        Ok((0, _)) => "",
        _ => "\r\n",
    };
    queue!(
        stdout,
        Print(prefix),
        SetForegroundColor(ikaros_accent_color()),
        Print(line),
        ResetColor,
        Print("\r\n\r\n")
    )?;
    stdout.flush()?;
    Ok(())
}

pub(super) fn clear_visible_terminal() -> Result<()> {
    if !io::stdout().is_terminal() {
        return Ok(());
    }
    let mut stdout = io::stdout();
    queue!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;
    stdout.flush()?;
    Ok(())
}

fn inline_turn_separator_text(width: usize) -> String {
    format!("\n{}\n\n", "─".repeat(width.max(24)))
}

fn inline_turn_worked_separator_text(width: usize, elapsed_ms: u128) -> String {
    format!(
        "\n{}\n\n",
        inline_labeled_separator(width, &inline_turn_worked_label(elapsed_ms))
    )
}

fn inline_turn_worked_label(elapsed_ms: u128) -> String {
    format!(" Worked for {} ", format_elapsed_duration(elapsed_ms))
}

fn inline_labeled_separator(width: usize, label: &str) -> String {
    let width = width.max(24);
    let label_width = terminal_display_width(label);
    if label_width >= width.saturating_sub(2) {
        return "─".repeat(width);
    }
    let left = 2usize;
    let right = width.saturating_sub(left + label_width);
    format!("{}{}{}", "─".repeat(left), label, "─".repeat(right))
}

fn format_elapsed_duration(elapsed_ms: u128) -> String {
    let mut seconds = elapsed_ms / 1000;
    let minutes = seconds / 60;
    seconds %= 60;
    if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

pub(super) fn print_inline_history_text(text: &str) -> Result<()> {
    let lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    print_inline_history_lines(&lines)
}

pub(super) fn print_inline_history_lines(lines: &[String]) -> Result<()> {
    if lines.is_empty() {
        return Ok(());
    }
    if io::stdout().is_terminal()
        && let Ok((_, cursor_y)) = crossterm::cursor::position()
        && cursor_y >= 2
    {
        let mut stdout = io::stdout();
        let width = terminal_width();
        let lines = wrap_inline_history_lines(lines, width);
        queue_inline_history_lines_insert(&mut stdout, &lines, cursor_y)?;
        stdout.flush()?;
        return Ok(());
    }
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

fn wrap_inline_history_lines(lines: &[String], width: usize) -> Vec<String> {
    lines
        .iter()
        .flat_map(|line| wrap_inline_history_line(line, width))
        .collect()
}

fn wrap_inline_history_line(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if ansi_stripped_display_width(line) <= width {
        return vec![line.to_owned()];
    }
    let continuation_prefix = inline_history_continuation_prefix(line);
    let continuation_width = terminal_display_width(&continuation_prefix);
    let mut chars = line.chars().peekable();
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            current.push_str(&collect_ansi_sequence(ch, &mut chars));
            continue;
        }
        let ch_width = terminal_char_width(ch);
        if !current.is_empty() && current_width.saturating_add(ch_width) > width {
            rows.push(std::mem::take(&mut current));
            current_width = 0;
            if continuation_width < width {
                current.push_str(&continuation_prefix);
                current_width = continuation_width;
            }
        }
        current.push(ch);
        current_width = current_width.saturating_add(ch_width);
    }
    if !current.is_empty() {
        rows.push(current);
    }
    if rows.is_empty() {
        rows.push(String::new());
    }
    rows
}

fn inline_history_continuation_prefix(line: &str) -> String {
    let visible = strip_ansi_sequences(line);
    if visible.starts_with("• ") || visible.starts_with("› ") {
        return "  ".to_owned();
    }
    visible
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .collect()
}

fn ansi_stripped_display_width(input: &str) -> usize {
    terminal_display_width(&strip_ansi_sequences(input))
}

fn strip_ansi_sequences(input: &str) -> String {
    let mut output = String::new();
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            let _ = collect_ansi_sequence(ch, &mut chars);
        } else {
            output.push(ch);
        }
    }
    output
}

fn collect_ansi_sequence(
    first: char,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> String {
    let mut sequence = String::new();
    sequence.push(first);
    if chars.peek().copied() == Some('[') {
        sequence.push(chars.next().expect("peeked CSI introducer"));
        for ch in chars.by_ref() {
            sequence.push(ch);
            if ('@'..='~').contains(&ch) {
                break;
            }
        }
    } else if let Some(ch) = chars.next() {
        sequence.push(ch);
    }
    sequence
}

fn queue_inline_history_lines_insert(
    stdout: &mut impl Write,
    lines: &[String],
    cursor_y: u16,
) -> Result<()> {
    queue!(stdout, SetScrollRegion(1..cursor_y))?;
    queue!(stdout, MoveTo(0, cursor_y.saturating_sub(1)))?;
    for line in lines {
        queue!(
            stdout,
            Print("\r\n"),
            Print(line),
            Clear(ClearType::UntilNewLine)
        )?;
    }
    queue!(stdout, ResetScrollRegion, MoveTo(0, cursor_y))?;
    Ok(())
}

fn submitted_user_message_lines(input: &str, width: usize) -> Vec<String> {
    let wrap_width = width.saturating_sub(2).max(1);
    let mut lines = Vec::new();
    let input = workbench::terminal_message(input)
        .trim_end_matches(['\r', '\n'])
        .to_owned();
    if input.is_empty() {
        return vec![String::new()];
    }
    for raw_line in input.split('\n') {
        let line = terminal_inline(raw_line);
        if line.is_empty() {
            lines.push(String::new());
            continue;
        }
        lines.extend(wrap_display_line(&line, wrap_width));
    }
    while lines.last().is_some_and(|line| line.trim().is_empty()) && lines.len() > 1 {
        lines.pop();
    }
    lines
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SubmittedUserMessageCellLine {
    text: String,
    kind: SubmittedUserMessageCellLineKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubmittedUserMessageCellLineKind {
    Spacer,
    First,
    Continuation,
}

fn submitted_user_message_cell_lines(
    input: &str,
    width: usize,
) -> Vec<SubmittedUserMessageCellLine> {
    let message_lines = submitted_user_message_lines(input, width);
    let mut rows = Vec::with_capacity(message_lines.len().saturating_add(2));
    rows.push(SubmittedUserMessageCellLine {
        text: String::new(),
        kind: SubmittedUserMessageCellLineKind::Spacer,
    });
    rows.extend(message_lines.into_iter().enumerate().map(|(index, text)| {
        SubmittedUserMessageCellLine {
            text,
            kind: if index == 0 {
                SubmittedUserMessageCellLineKind::First
            } else {
                SubmittedUserMessageCellLineKind::Continuation
            },
        }
    }));
    rows.push(SubmittedUserMessageCellLine {
        text: String::new(),
        kind: SubmittedUserMessageCellLineKind::Spacer,
    });
    rows
}

fn wrap_display_line(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if line.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for ch in terminal_inline(line).chars() {
        let ch_width = terminal_char_width(ch);
        if !current.is_empty() && current_width.saturating_add(ch_width) > width {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width = current_width.saturating_add(ch_width);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn clear_inline_composer_rows(
    rows: u16,
    cursor_row_from_top: u16,
    composer_top: Option<u16>,
) -> Result<()> {
    if rows == 0 {
        return Ok(());
    }
    let mut stdout = io::stdout();
    let composer_top = composer_top.or_else(|| {
        crossterm::cursor::position()
            .ok()
            .map(|(_, y)| y.saturating_sub(cursor_row_from_top))
    });
    if let Some(top) = composer_top {
        queue!(stdout, MoveTo(0, top))?;
    } else if cursor_row_from_top > 0 {
        queue!(stdout, MoveUp(cursor_row_from_top))?;
    }
    for index in 0..rows {
        if index > 0 {
            queue!(stdout, Print("\r\n"))?;
        }
        queue!(stdout, MoveToColumn(0), Clear(ClearType::CurrentLine))?;
    }
    if let Some(top) = composer_top {
        queue!(stdout, MoveTo(0, top))?;
    } else if rows > 1 {
        queue!(stdout, MoveUp(rows - 1))?;
    }
    queue!(stdout, MoveToColumn(0))?;
    stdout.flush()?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InlineComposerLine {
    text: String,
    style: InlineComposerLineStyle,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct WorkbenchLineEditorRenderState {
    rows: u16,
    viewport_anchored: bool,
    cursor_row_from_top: u16,
    composer_top: Option<u16>,
}

impl WorkbenchLineEditorRenderState {
    fn clear_rows_before_history_insert(&mut self) -> Result<()> {
        clear_inline_composer_rows(self.rows, self.cursor_row_from_top, self.composer_top)?;
        self.rows = 0;
        self.viewport_anchored = false;
        self.cursor_row_from_top = 0;
        self.composer_top = None;
        Ok(())
    }

    #[cfg(test)]
    fn mark_rows_for_test(&mut self, rows: u16) {
        self.rows = rows;
    }

    #[cfg(test)]
    fn mark_cursor_row_from_top_for_test(&mut self, cursor_row_from_top: u16) {
        self.cursor_row_from_top = cursor_row_from_top;
    }

    #[cfg(test)]
    fn mark_composer_top_for_test(&mut self, composer_top: u16) {
        self.composer_top = Some(composer_top);
    }

    #[cfg(test)]
    fn mark_viewport_anchored_for_test(&mut self) {
        self.viewport_anchored = true;
    }

    #[cfg(test)]
    fn rows(&self) -> u16 {
        self.rows
    }

    #[cfg(test)]
    fn viewport_anchored(&self) -> bool {
        self.viewport_anchored
    }

    #[cfg(test)]
    fn composer_top(&self) -> Option<u16> {
        self.composer_top
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InlineComposerLineStyle {
    Hint,
    SelectedHint,
    Prompt,
    Placeholder,
    Footer,
}

fn inline_composer_lines(
    input_state: &WorkbenchInputState,
    ui: &WorkbenchLineInputUi,
    width: usize,
) -> Vec<InlineComposerLine> {
    let mut lines = Vec::new();
    let prompt_style = if input_state.buffer().is_empty() {
        InlineComposerLineStyle::Placeholder
    } else {
        InlineComposerLineStyle::Prompt
    };
    lines.push(InlineComposerLine {
        text: String::new(),
        style: InlineComposerLineStyle::Prompt,
    });
    lines.extend(
        inline_composer_prompt_lines(input_state, width)
            .into_iter()
            .map(|text| InlineComposerLine {
                text,
                style: prompt_style,
            }),
    );
    lines.push(InlineComposerLine {
        text: String::new(),
        style: InlineComposerLineStyle::Prompt,
    });
    if input_state.history_search_active() {
        lines.push(InlineComposerLine {
            text: format!("  history search: {}", input_state.history_search_summary()),
            style: InlineComposerLineStyle::Hint,
        });
        for candidate in input_state
            .history_search_candidates(INLINE_COMPOSER_HINT_LIMIT)
            .into_iter()
        {
            lines.push(InlineComposerLine {
                text: format!("  {}", terminal_inline(&candidate)),
                style: InlineComposerLineStyle::Hint,
            });
        }
    } else {
        let completion_query = input_state.completion_query();
        if !completion_query.is_empty() {
            let completions = input_state.completion_candidates();
            let selected_completion = input_state
                .completion_selected()
                .map(str::to_owned)
                .or_else(|| completions.first().cloned());
            lines.extend(inline_completion_popup_lines(
                &completion_query,
                completions,
                selected_completion.as_deref(),
                width,
            ));
        }
    }
    let model_width = terminal_display_width(&ui.model_label);
    let path_width = width.saturating_sub(model_width + 5);
    let workspace_label = compact_path_label(&ui.workspace_label, path_width);
    lines.push(InlineComposerLine {
        text: format!("{} · {}", ui.model_label, workspace_label),
        style: InlineComposerLineStyle::Footer,
    });
    lines
}

fn inline_composer_prompt_lines(input_state: &WorkbenchInputState, width: usize) -> Vec<String> {
    if input_state.buffer().is_empty() {
        return vec![fit_terminal_text("› Ask Ikaros to do anything", width)];
    }
    let content_width = width.saturating_sub(2).max(1);
    let mut rows = Vec::new();
    for raw_line in input_state.buffer().split('\n') {
        let wrapped = wrap_display_line(raw_line, content_width);
        for segment in wrapped {
            let prefix = if rows.is_empty() { "› " } else { "  " };
            rows.push(fit_terminal_text(&format!("{prefix}{segment}"), width));
        }
    }
    rows
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InlineComposerCursor {
    row: u16,
    column: u16,
}

fn inline_composer_cursor(input_state: &WorkbenchInputState, width: usize) -> InlineComposerCursor {
    let content_width = width.saturating_sub(2).max(1);
    let max_column = width.saturating_sub(1) as u16;
    if input_state.buffer().is_empty() {
        return InlineComposerCursor {
            row: 1,
            column: 2.min(max_column),
        };
    }

    let before_cursor = input_state
        .buffer()
        .chars()
        .take(input_state.cursor())
        .collect::<String>();
    let mut row = 0usize;
    let lines = before_cursor.split('\n').collect::<Vec<_>>();
    for (index, raw_line) in lines.iter().enumerate() {
        let segments = wrap_display_line(raw_line, content_width);
        if index + 1 == lines.len() {
            let segment = segments.last().map(String::as_str).unwrap_or_default();
            let column = 2usize.saturating_add(terminal_display_width(segment));
            return InlineComposerCursor {
                row: (row + segments.len().saturating_sub(1) + 1) as u16,
                column: (column as u16).min(max_column),
            };
        }
        row += segments.len();
    }

    InlineComposerCursor {
        row: 1,
        column: 2.min(max_column),
    }
}

fn inline_completion_popup_lines(
    query: &str,
    completions: Vec<String>,
    selected_completion: Option<&str>,
    width: usize,
) -> Vec<InlineComposerLine> {
    let popup_width = inline_popup_width(width);
    let specs = slash_completion_popup_specs(&completions);
    let mut popup = SlashCommandPopup::new(specs);
    popup.update_filter(query);
    if let Some(selected) = selected_completion {
        let selected = selected.trim_start_matches('/');
        if let Some(index) = popup
            .filtered_items()
            .iter()
            .position(|item| item.command_name() == selected)
        {
            while popup.state().selected_index() < index {
                popup.move_down();
            }
            while popup.state().selected_index() > index {
                popup.move_up();
            }
        } else {
            popup.select_exact_match();
        }
    } else {
        popup.select_exact_match();
    }
    let mut lines = vec![InlineComposerLine {
        text: codex_popup_border('╭', '╮', "Slash Commands", popup_width),
        style: InlineComposerLineStyle::Hint,
    }];
    for rendered in popup.render_lines(popup_width.saturating_sub(4), INLINE_COMPOSER_HINT_LIMIT) {
        let selected = rendered.trim_start().starts_with('›');
        lines.push(InlineComposerLine {
            text: codex_popup_item_rendered_line(&rendered, popup_width),
            style: if selected {
                InlineComposerLineStyle::SelectedHint
            } else {
                InlineComposerLineStyle::Hint
            },
        });
    }
    lines.push(InlineComposerLine {
        text: codex_popup_border('╰', '╯', "", popup_width),
        style: InlineComposerLineStyle::Hint,
    });
    lines
}

fn slash_completion_popup_specs(completions: &[String]) -> Vec<SlashCommandSpec> {
    if completions.is_empty() {
        return workbench::slash_command_palette_items(None, usize::MAX)
            .into_iter()
            .map(slash_palette_item_spec)
            .collect();
    }
    completions
        .iter()
        .map(|completion| slash_completion_spec(completion))
        .collect()
}

fn slash_completion_spec(command: &str) -> SlashCommandSpec {
    let command = terminal_inline(command);
    let normalized = command.trim_start_matches('/');
    let item = workbench::slash_command_palette_items(Some(&command), 1)
        .into_iter()
        .find(|item| item.name.trim_start_matches('/') == normalized);
    if let Some(item) = item {
        slash_palette_item_spec(item)
    } else {
        SlashCommandSpec::new(command, "")
    }
}

fn slash_palette_item_spec(item: workbench::SlashCommandPaletteItem) -> SlashCommandSpec {
    let hidden_alias = item.tags.split(',').any(|tag| tag.trim() == "alias");
    SlashCommandSpec::new(item.name, terminal_inline(item.summary))
        .category(item.effect)
        .hidden_alias(hidden_alias)
        .available_during_task(item.effect != "session-mutation")
}

fn inline_popup_width(width: usize) -> usize {
    let target = width.saturating_sub(2).max(1);
    target.max(20.min(width.max(1)))
}

fn codex_popup_border(left: char, right: char, title: &str, width: usize) -> String {
    let width = width.max(4);
    let label = if title.trim().is_empty() {
        String::new()
    } else {
        format!("─{}─", title.trim())
    };
    let label_width = terminal_display_width(&label);
    let rule_width = width.saturating_sub(2 + label_width);
    format!("{left}{label}{}{right}", "─".repeat(rule_width))
}

fn codex_popup_item_rendered_line(line: &str, width: usize) -> String {
    let inner_width = width.saturating_sub(4).max(1);
    format!("│ {} │", pad_terminal_text(line, inner_width))
}

fn compact_path_label(path: &str, max_width: usize) -> String {
    let max_width = max_width.max(1);
    let mut path = terminal_inline(path);
    if let Some(home) = std::env::var_os("HOME").map(|home| home.to_string_lossy().to_string())
        && !home.is_empty()
        && path == home
    {
        path = "~".to_owned();
    } else if let Some(home) =
        std::env::var_os("HOME").map(|home| home.to_string_lossy().to_string())
        && !home.is_empty()
        && path.starts_with(&(home.clone() + "/"))
    {
        path = format!("~{}", &path[home.len()..]);
    }
    compact_middle_text(&path, max_width)
}

fn compact_middle_text(input: &str, max_width: usize) -> String {
    if terminal_display_width(input) <= max_width {
        return input.to_owned();
    }
    if max_width <= 1 {
        return "…".to_owned();
    }
    if max_width <= 4 {
        return take_display_prefix(input, max_width);
    }
    let marker = "…";
    let keep = max_width.saturating_sub(terminal_display_width(marker));
    let head = keep / 2;
    let tail = keep.saturating_sub(head);
    let prefix = take_display_prefix(input, head);
    let suffix = take_display_suffix(input, tail);
    format!("{prefix}{marker}{suffix}")
}

fn terminal_width() -> usize {
    terminal_dimensions().0
}

fn terminal_dimensions() -> (usize, u16) {
    terminal_size()
        .map(|(width, height)| (usize::from(width).max(20), height.max(1)))
        .unwrap_or((80, 24))
}

fn inline_viewport_spacer_rows(_cursor_y: u16, _terminal_height: u16, _composer_rows: u16) -> u16 {
    0
}

fn fit_terminal_text(input: &str, width: usize) -> String {
    let width = width.max(1);
    let mut output = String::new();
    let mut used = 0usize;
    for ch in terminal_inline(input).chars() {
        let ch_width = terminal_char_width(ch);
        if used.saturating_add(ch_width) > width {
            break;
        }
        output.push(ch);
        used = used.saturating_add(ch_width);
    }
    output
}

fn pad_terminal_text(input: &str, width: usize) -> String {
    let width = width.max(1);
    let mut output = fit_terminal_text(input, width);
    let count = terminal_display_width(&output);
    if count < width {
        output.push_str(&" ".repeat(width - count));
    }
    output
}

fn terminal_display_width(input: &str) -> usize {
    UnicodeWidthStr::width(input)
}

fn terminal_char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

fn take_display_prefix(input: &str, max_width: usize) -> String {
    let mut output = String::new();
    let mut used = 0usize;
    for ch in input.chars() {
        let ch_width = terminal_char_width(ch);
        if used.saturating_add(ch_width) > max_width {
            break;
        }
        output.push(ch);
        used = used.saturating_add(ch_width);
    }
    output
}

fn take_display_suffix(input: &str, max_width: usize) -> String {
    let mut chars = Vec::new();
    let mut used = 0usize;
    for ch in input.chars().rev() {
        let ch_width = terminal_char_width(ch);
        if used.saturating_add(ch_width) > max_width {
            break;
        }
        chars.push(ch);
        used = used.saturating_add(ch_width);
    }
    chars.into_iter().rev().collect()
}

pub(super) fn handle_workbench_input_control(
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

pub(super) fn read_multiline_message() -> Result<Option<String>> {
    let mut message = String::new();
    let mut line = String::new();
    loop {
        print!("... ");
        io::stdout().flush()?;
        line.clear();
        if io::stdin().read_line(&mut line)? == 0 {
            return if message.is_empty() {
                Ok(None)
            } else {
                Ok(Some(message))
            };
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed == workbench::MULTILINE_TERMINATOR {
            return Ok(Some(message));
        }
        message.push_str(trimmed);
        message.push('\n');
    }
}

pub(super) fn read_bracketed_paste_message(first_line: &str) -> Result<Option<String>> {
    let mut message = String::new();
    let mut current = first_line
        .strip_prefix(BRACKETED_PASTE_START)
        .unwrap_or(first_line)
        .to_owned();
    loop {
        if let Some((before_end, _after_end)) = current.split_once(BRACKETED_PASTE_END) {
            message.push_str(before_end.trim_end_matches(['\n', '\r']));
            return Ok(Some(message));
        }
        message.push_str(current.trim_end_matches(['\n', '\r']));
        message.push('\n');
        current.clear();
        if io::stdin().read_line(&mut current)? == 0 {
            return if message.is_empty() {
                Ok(None)
            } else {
                Ok(Some(message))
            };
        }
    }
}

pub(super) struct WorkbenchTerminalInputSessionGuard {
    keyboard_enhancements_pushed: bool,
}

impl WorkbenchTerminalInputSessionGuard {
    pub(super) fn enable() -> Result<Self> {
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

struct FullscreenRawModeGuard {
    terminal_modes_owned: bool,
    keyboard_enhancements_pushed: bool,
}

impl FullscreenRawModeGuard {
    fn enable_for_line_input(terminal_modes_owned: bool) -> Result<Self> {
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

    #[cfg(test)]
    fn owns_terminal_modes(&self) -> bool {
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
struct SetScrollRegion(std::ops::Range<u16>);

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
struct ResetScrollRegion;

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
    use super::*;

    #[test]
    fn submitted_user_message_lines_match_codex_history_cell_shape() {
        let lines = submitted_user_message_lines("one two three four five", 12);

        assert_eq!(
            lines,
            vec![
                "one two th".to_owned(),
                "ree four f".to_owned(),
                "ive".to_owned()
            ]
        );
    }

    #[test]
    fn submitted_user_message_cell_lines_add_codex_like_padding_rows() {
        let lines = submitted_user_message_cell_lines("one two three four five", 12);

        assert_eq!(
            lines,
            vec![
                SubmittedUserMessageCellLine {
                    text: String::new(),
                    kind: SubmittedUserMessageCellLineKind::Spacer
                },
                SubmittedUserMessageCellLine {
                    text: "one two th".to_owned(),
                    kind: SubmittedUserMessageCellLineKind::First
                },
                SubmittedUserMessageCellLine {
                    text: "ree four f".to_owned(),
                    kind: SubmittedUserMessageCellLineKind::Continuation
                },
                SubmittedUserMessageCellLine {
                    text: "ive".to_owned(),
                    kind: SubmittedUserMessageCellLineKind::Continuation
                },
                SubmittedUserMessageCellLine {
                    text: String::new(),
                    kind: SubmittedUserMessageCellLineKind::Spacer
                },
            ]
        );
    }

    #[test]
    fn inline_turn_separator_has_surrounding_blank_lines() {
        assert_eq!(
            inline_turn_separator_text(32),
            "\n────────────────────────────────\n\n"
        );
    }

    #[test]
    fn inline_turn_worked_separator_matches_codex_shape() {
        let rendered = inline_turn_worked_separator_text(72, 660_000);

        assert!(rendered.starts_with("\n── Worked for 11m 00s "));
        assert!(rendered.ends_with("\n\n"));
        assert_eq!(rendered.lines().nth(1).unwrap().chars().count(), 72);
    }

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
        assert!(rows[1].starts_with("› Ask Ikaros while I work"));
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
        assert!(rows[1].starts_with("› next"));
        assert!(rows[1].contains("queued 2"));
        assert!(!rows[1].ends_with(' '));
        assert_eq!(rows[2], "");
        assert!(output.contains("\x1b[K"));
        assert!(output.contains("\x1b[K\x1b[1G"));
        assert!(output.contains("\x1b[K\x1b[0m"));
        assert!(output.ends_with("\x1b[1A\x1b[7G"));
    }

    #[test]
    fn submitted_user_message_cell_paints_full_background_without_real_padding() {
        let mut output = Vec::new();
        let line = SubmittedUserMessageCellLine {
            text: "hello".to_owned(),
            kind: SubmittedUserMessageCellLineKind::First,
        };

        queue_submitted_user_message_cell_line(&mut output, &line, 32)
            .expect("submitted user line");
        let output = String::from_utf8(output).expect("submitted user output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert_eq!(visible, "› hello");
        assert!(output.contains("\x1b[K\x1b[1G"));
        assert!(output.ends_with("\x1b[0m"));
        assert!(!visible.ends_with(' '));
    }

    #[test]
    fn submitted_user_message_spacer_paints_visual_padding_without_spaces() {
        let mut output = Vec::new();
        let line = SubmittedUserMessageCellLine {
            text: String::new(),
            kind: SubmittedUserMessageCellLineKind::Spacer,
        };

        queue_submitted_user_message_cell_line(&mut output, &line, 32)
            .expect("submitted user spacer");
        let output = String::from_utf8(output).expect("submitted user output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert!(output.contains("\x1b[K\x1b[0m"));
        assert_eq!(visible, "");
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

    #[test]
    fn submitted_user_message_lines_preserve_explicit_newlines_and_strip_controls() {
        let lines = submitted_user_message_lines("first\x1b[<35;1;1M\nsecond\n", 20);

        assert_eq!(lines, vec!["first".to_owned(), "second".to_owned()]);
    }

    #[test]
    fn submitted_user_message_lines_strip_private_mode_sequences() {
        let lines = submitted_user_message_lines("\x1b[?1000l\x1b[?1006lhello", 20);

        assert_eq!(lines, vec!["hello".to_owned()]);
    }

    #[test]
    fn submitted_user_message_line_styles_prompt_marker_separately() {
        let mut output = Vec::new();

        queue_submitted_user_message_cell_line(
            &mut output,
            &SubmittedUserMessageCellLine {
                text: "hello".to_owned(),
                kind: SubmittedUserMessageCellLineKind::First,
            },
            20,
        )
        .expect("submitted user line");
        let output = String::from_utf8(output).expect("submitted output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert!(output.contains("› "));
        assert!(output.contains("hello"));
        assert!(output.contains("\x1b[0m"));
        assert!(output.contains("\x1b[K"));
        assert!(output.contains("\x1b[K\x1b[1G"));
        assert_eq!(visible, "› hello");
        assert!(!visible.ends_with(' '));
    }

    #[test]
    fn submitted_user_message_continuation_line_is_indented_full_width_cell() {
        let mut output = Vec::new();

        queue_submitted_user_message_cell_line(
            &mut output,
            &SubmittedUserMessageCellLine {
                text: "wrapped".to_owned(),
                kind: SubmittedUserMessageCellLineKind::Continuation,
            },
            14,
        )
        .expect("submitted continuation line");
        let output = String::from_utf8(output).expect("submitted output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert_eq!(visible, "  wrapped");
        assert!(!visible.ends_with(' '));
        assert!(output.contains("\x1b[K\x1b[1G"));
    }

    #[test]
    fn submitted_user_message_spacer_line_paints_visual_padding() {
        let mut output = Vec::new();

        queue_submitted_user_message_cell_line(
            &mut output,
            &SubmittedUserMessageCellLine {
                text: String::new(),
                kind: SubmittedUserMessageCellLineKind::Spacer,
            },
            8,
        )
        .expect("submitted spacer line");
        let output = String::from_utf8(output).expect("submitted output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert!(output.contains("\x1b[K\x1b[0m"));
        assert_eq!(visible, "");
    }

    #[test]
    fn submitted_user_message_history_insert_uses_scroll_region_above_composer() {
        let mut output = Vec::new();

        queue_submitted_user_message_history_insert(&mut output, "hello\nwrapped", 20, 10)
            .expect("history insert");
        let output = String::from_utf8(output).expect("history insert output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert!(output.contains("\x1b[1;10r"));
        assert!(output.contains("\x1b[10;1H"));
        assert_eq!(output.matches("\r\n").count(), 4);
        assert!(output.contains("› "));
        assert!(output.contains("hello"));
        assert!(output.contains("wrapped"));
        assert!(output.contains("\x1b[r"));
        assert!(output.ends_with("\x1b[11;1H"));

        let visible_rows = visible.split("\r\n").collect::<Vec<_>>();
        assert_eq!(visible_rows.len(), 5);
        assert_eq!(visible_rows[0], "");
        assert_eq!(visible_rows[1], "");
        assert_eq!(visible_rows[2], "› hello");
        assert_eq!(visible_rows[3], "  wrapped");
        assert_eq!(visible_rows[4], "");
    }

    #[test]
    fn inline_composer_line_clears_before_repainting() {
        let mut output = Vec::new();
        let line = InlineComposerLine {
            text: "› hello".to_owned(),
            style: InlineComposerLineStyle::Prompt,
        };

        queue_inline_composer_line(&mut output, &line, 20).expect("queue line");
        let output = String::from_utf8(output).expect("line output is utf8");

        assert!(
            output.starts_with("\x1b[2K"),
            "composer line should clear before repainting: {output:?}"
        );
    }

    #[test]
    fn inline_composer_placeholder_uses_codex_like_muted_prompt() {
        let input_state = WorkbenchInputState::default();
        let ui = WorkbenchLineInputUi::new("mock-chat", "/tmp/project", false);
        let width = 36;
        let lines = inline_composer_lines(&input_state, &ui, width);
        let prompt = lines
            .iter()
            .find(|line| line.style == InlineComposerLineStyle::Placeholder)
            .expect("placeholder line");

        assert_eq!(prompt.style, InlineComposerLineStyle::Placeholder);
        assert_eq!(prompt.text, "› Ask Ikaros to do anything");

        let mut output = Vec::new();
        queue_inline_composer_line(&mut output, prompt, width).expect("queue placeholder");
        let output = String::from_utf8(output).expect("placeholder output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert!(output.starts_with("\x1b[2K"));
        assert!(output.contains("› "));
        assert!(output.contains("Ask Ikaros to do anything"));
        assert!(output.contains("\x1b[K"));
        assert!(output.contains("\x1b[K\x1b[1G"));
        assert!(!visible.ends_with(' '));
    }

    #[test]
    fn inline_composer_input_block_has_padding_above_and_below_prompt() {
        let input_state = WorkbenchInputState::default();
        let ui = WorkbenchLineInputUi::new("mock-chat", "/tmp/project", false);
        let width = 36;
        let lines = inline_composer_lines(&input_state, &ui, width);
        let footer_index = lines
            .iter()
            .position(|line| line.style == InlineComposerLineStyle::Footer)
            .expect("footer line");
        let input_block = &lines[..footer_index];

        assert!(input_block.len() >= 3);
        assert_eq!(
            input_block.first().expect("top padding"),
            &InlineComposerLine {
                text: String::new(),
                style: InlineComposerLineStyle::Prompt,
            }
        );
        assert_eq!(
            input_block.last().expect("bottom padding"),
            &InlineComposerLine {
                text: String::new(),
                style: InlineComposerLineStyle::Prompt,
            }
        );

        let mut output = Vec::new();
        queue_inline_composer_line(
            &mut output,
            input_block.first().expect("top padding"),
            width,
        )
        .expect("queue padding");
        let output = String::from_utf8(output).expect("padding output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert!(!visible.contains('›'));
        assert_eq!(terminal_display_width(&visible), 0);
        assert!(output.contains("\x1b[K"));
        assert!(output.contains("\x1b[K\x1b[0m"));
    }

    #[test]
    fn inline_composer_prompt_line_paints_full_background_without_real_padding() {
        let line = InlineComposerLine {
            text: "› Ask Ikaros to do anything".to_owned(),
            style: InlineComposerLineStyle::Placeholder,
        };
        let mut output = Vec::new();

        queue_inline_composer_line(&mut output, &line, 48).expect("queue prompt line");
        let output = String::from_utf8(output).expect("prompt output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert!(output.contains("\x1b[K\x1b[1G"));
        assert_eq!(visible, "› Ask Ikaros to do anything");
        assert!(!visible.ends_with(' '));
    }

    #[test]
    fn codex_input_cell_text_line_uses_ansi_background_not_real_padding() {
        let mut output = Vec::new();

        queue_codex_input_cell_text_line(&mut output, [(Color::White, "› "), (Color::Grey, "ask")])
            .expect("input cell line");
        let output = String::from_utf8(output).expect("input cell output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert!(output.contains("\x1b[K\x1b[1G"));
        assert_eq!(visible, "› ask");
        assert!(!visible.ends_with(' '));
        assert!(!output.contains(" ".repeat(8).as_str()));
    }

    #[test]
    fn worked_separator_does_not_stack_blank_rows_after_streamed_text() {
        let combined = format!(
            "{}{}",
            "• 你好\n\n  今天有什么可以帮你的吗？\n",
            inline_turn_worked_separator_text(72, 2_000)
        );

        assert!(!combined.contains("\n\n\n"), "{combined:?}");
    }

    #[test]
    fn slash_completion_spec_uses_palette_summary() {
        let spec = slash_completion_spec("/model");

        assert_eq!(spec.slash_name(), "/model");
        assert!(spec.description.contains("inspect active model descriptor"));
    }

    #[test]
    fn inline_intro_starts_with_codex_like_green_brand_line() {
        let ui = WorkbenchLineInputUi::new("mock-chat", "/tmp/project", true);
        let mut output = Vec::new();

        queue_workbench_inline_intro(&mut output, &ui, 80).expect("intro renders");
        let output = String::from_utf8(output).expect("intro output is utf8");

        assert!(output.starts_with("\x1b["));
        assert!(output.contains("ikaros"));
        assert!(output.contains(">_ Ikaros"));
        assert!(output.contains("/model"));
        assert!(!output.contains("Tip:"));
        assert!(!output.contains("/mcp to list configured MCP tools"));
    }

    #[test]
    fn raw_mode_guard_can_borrow_session_terminal_modes() {
        let guard = FullscreenRawModeGuard {
            terminal_modes_owned: false,
            keyboard_enhancements_pushed: false,
        };

        assert!(!guard.owns_terminal_modes());
    }

    #[test]
    fn inline_composer_footer_compacts_long_workspace_path() {
        let input_state = WorkbenchInputState::default();
        let ui = WorkbenchLineInputUi::new(
            "mock-chat",
            "/tmp/some/really/long/workspace/path/that/does/not/fit",
            false,
        );
        let lines = inline_composer_lines(&input_state, &ui, 40);
        let footer = lines.last().expect("footer line");

        assert_eq!(footer.style, InlineComposerLineStyle::Footer);
        assert!(footer.text.contains("mock-chat · "));
        assert!(!footer.text.starts_with(' '));
        assert!(footer.text.contains('…'));
        assert!(footer.text.chars().count() <= 40);
    }

    #[test]
    fn inline_composer_footer_sanitizes_labels_and_paints_stable_width() {
        let input_state = WorkbenchInputState::default();
        let width = 32;
        let ui = WorkbenchLineInputUi::new(
            "mock\x1b[31m-chat",
            "/tmp/\x1b[?1000hproject-with-long-name",
            false,
        );
        let lines = inline_composer_lines(&input_state, &ui, width);
        let footer = lines.last().expect("footer line");

        assert_eq!(footer.style, InlineComposerLineStyle::Footer);
        assert!(!footer.text.contains('\u{1b}'));
        assert!(terminal_display_width(&footer.text) <= width);

        let mut output = Vec::new();
        queue_inline_composer_line(&mut output, footer, width).expect("queue footer");
        let output = String::from_utf8(output).expect("footer output is utf8");
        let visible = strip_ansi_sequences(&output);

        assert!(output.starts_with("\x1b[2K"));
        assert!(output.contains("\x1b[K"));
        assert!(output.contains("\x1b[0m\x1b[K"));
        assert!(output.ends_with("\x1b[K"));
        assert!(terminal_display_width(&visible) <= width);
        assert!(!visible.ends_with(' '));
        assert!(!visible.contains("[31m"));
        assert!(!visible.contains("[?1000h"));
    }

    #[test]
    fn inline_composer_marks_first_slash_completion_selected() {
        let mut input_state = WorkbenchInputState::default();
        input_state.set_buffer("/mo");
        let ui = WorkbenchLineInputUi::new("mock-chat", "/tmp/project", false);
        let lines = inline_composer_lines(&input_state, &ui, 80);

        let popup_title = lines
            .iter()
            .find(|line| line.text.contains("Slash Commands"))
            .expect("slash command popup title");
        assert_eq!(popup_title.style, InlineComposerLineStyle::Hint);

        let model_line = lines
            .iter()
            .find(|line| line.text.contains("/model"))
            .expect("model completion line");
        assert_eq!(model_line.style, InlineComposerLineStyle::SelectedHint);
        assert!(model_line.text.contains("› /model"));
        assert!(model_line.text.starts_with("│ "));
    }

    #[test]
    fn inline_composer_shows_default_slash_popup_for_bare_slash() {
        let mut input_state = WorkbenchInputState::default();
        input_state.set_buffer("/");
        let ui = WorkbenchLineInputUi::new("mock-chat", "/tmp/project", false);
        let lines = inline_composer_lines(&input_state, &ui, 80);

        assert!(
            lines
                .iter()
                .any(|line| line.text.contains("Slash Commands"))
        );
        assert!(lines.iter().any(|line| line.text.contains('/')));
        assert!(!lines.iter().any(|line| line.text.contains("/new")));
        assert!(!lines.iter().any(|line| line.text.contains("/exit")));
    }

    #[test]
    fn inline_composer_reads_selected_completion_from_input_state() {
        let mut input_state = WorkbenchInputState::default();
        input_state.set_buffer("/");
        input_state.apply(WorkbenchInputAction::CompletionNext);
        let selected = input_state
            .completion_selected()
            .expect("selected completion after cycling")
            .to_owned();
        let ui = WorkbenchLineInputUi::new("mock-chat", "/tmp/project", false);
        let lines = inline_composer_lines(&input_state, &ui, 80);

        let selected_line = lines
            .iter()
            .find(|line| line.style == InlineComposerLineStyle::SelectedHint)
            .expect("selected slash command line");
        assert!(selected_line.text.contains(&selected));
    }

    #[test]
    fn inline_composer_reads_paged_completion_from_input_state() {
        let mut input_state = WorkbenchInputState::default();
        input_state.set_buffer("/");
        input_state.apply(WorkbenchInputAction::CompletionPageNext);
        let selected = input_state
            .completion_selected()
            .expect("selected completion after page cycling")
            .to_owned();
        let ui = WorkbenchLineInputUi::new("mock-chat", "/tmp/project", false);
        let lines = inline_composer_lines(&input_state, &ui, 80);

        let selected_line = lines
            .iter()
            .find(|line| line.style == InlineComposerLineStyle::SelectedHint)
            .expect("selected paged slash command line");
        assert!(selected_line.text.contains(&selected));
    }

    #[test]
    fn workbench_tab_completes_selected_slash_command_with_space() {
        let mut input_state = WorkbenchInputState::default();
        input_state.set_buffer("/");
        input_state.apply(WorkbenchInputAction::CompletionNext);
        let selected = input_state
            .completion_selected()
            .expect("selected completion before complete")
            .to_owned();

        let completed = input_state
            .apply(WorkbenchInputAction::Complete)
            .expect("tab completion applies selected command");

        assert_eq!(completed, format!("{selected} "));
    }

    #[test]
    fn inline_composer_renders_no_matching_slash_commands() {
        let mut input_state = WorkbenchInputState::default();
        input_state.set_buffer("/zzzz");
        let ui = WorkbenchLineInputUi::new("mock-chat", "/tmp/project", false);
        let lines = inline_composer_lines(&input_state, &ui, 80);

        assert!(
            lines
                .iter()
                .any(|line| line.text.contains("Slash Commands"))
        );
        assert!(
            lines
                .iter()
                .any(|line| line.text.contains("no matching commands"))
        );
    }

    #[test]
    fn inline_completion_popup_hides_aliases_in_default_filter() {
        let lines = inline_completion_popup_lines(
            "/",
            vec!["/clear".to_owned(), "/new".to_owned()],
            Some("/clear"),
            80,
        );

        assert!(lines.iter().any(|line| line.text.contains("/clear")));
        assert!(!lines.iter().any(|line| line.text.contains("/new")));
        assert!(slash_completion_spec("/new").hidden_alias);
    }

    #[test]
    fn inline_completion_popup_wraps_on_narrow_width() {
        let popup_width = inline_popup_width(30);
        let lines = inline_completion_popup_lines(
            "/commands",
            vec!["/commands".to_owned()],
            Some("/commands"),
            30,
        );

        assert_eq!(popup_width, 28);
        assert!(lines.len() > 3);
        assert!(
            lines
                .iter()
                .all(|line| terminal_display_width(&line.text) <= popup_width)
        );
    }

    #[test]
    fn inline_completion_popup_leaves_terminal_edge_margin() {
        assert_eq!(inline_popup_width(120), 118);
        assert_eq!(inline_popup_width(80), 78);
        assert_eq!(inline_popup_width(30), 28);
        assert_eq!(inline_popup_width(20), 20);
    }

    #[test]
    fn inline_viewport_spacer_keeps_composer_in_scrollback_near_current_output() {
        assert_eq!(inline_viewport_spacer_rows(4, 24, 2), 0);
        assert_eq!(inline_viewport_spacer_rows(0, 24, 2), 0);
        assert_eq!(inline_viewport_spacer_rows(22, 24, 2), 0);
        assert_eq!(inline_viewport_spacer_rows(23, 24, 2), 0);
        assert_eq!(inline_viewport_spacer_rows(0, 3, 5), 0);
    }

    #[test]
    fn inline_composer_repaint_keeps_stored_anchor_when_cursor_drifted() {
        assert_eq!(
            inline_composer_repaint_top(Some(10), 4, 1, true, Some(30)),
            Some(10)
        );
        assert_eq!(
            inline_composer_repaint_top(None, 4, 1, true, Some(30)),
            Some(29)
        );
    }

    #[test]
    fn line_editor_state_reanchors_after_history_insert() {
        let mut state = WorkbenchLineEditorRenderState::default();
        state.mark_rows_for_test(0);
        state.mark_cursor_row_from_top_for_test(1);
        state.mark_composer_top_for_test(8);
        state.mark_viewport_anchored_for_test();

        state
            .clear_rows_before_history_insert()
            .expect("clear empty composer rows");

        assert_eq!(state.rows(), 0);
        assert!(!state.viewport_anchored());
        assert_eq!(state.composer_top(), None);
    }

    #[test]
    fn inline_history_lines_insert_uses_scroll_region_and_restores_cursor() {
        let mut output = Vec::new();
        queue_inline_history_lines_insert(
            &mut output,
            &["• Explored".to_owned(), "  • Read SKILL.md".to_owned()],
            8,
        )
        .expect("history lines insert");
        let output = String::from_utf8(output).expect("history insert output is utf8");

        assert!(output.contains("\x1b[1;8r"));
        assert!(output.contains("• Explored"));
        assert!(output.contains("  • Read SKILL.md"));
        assert!(output.contains("\x1b[r"));
        assert!(output.ends_with("\x1b[9;1H"));
    }

    #[test]
    fn inline_history_lines_are_wrapped_before_scrollback_insert() {
        let lines = wrap_inline_history_lines(
            &["• This is a deliberately long assistant line".to_owned()],
            18,
        );

        assert_eq!(
            lines,
            vec![
                "• This is a delibe".to_owned(),
                "  rately long assi".to_owned(),
                "  stant line".to_owned()
            ]
        );
    }

    #[test]
    fn inline_history_wrapping_treats_ansi_sequences_as_zero_width() {
        let lines = wrap_inline_history_lines(&["\x1b[32m•\x1b[0m 你好世界abcdef".to_owned()], 10);

        assert_eq!(
            lines,
            vec![
                "\x1b[32m•\x1b[0m 你好世界".to_owned(),
                "  abcdef".to_owned()
            ]
        );
    }

    #[test]
    fn inline_composer_expands_multiline_input_like_textarea() {
        let mut input_state = WorkbenchInputState::default();
        input_state.set_buffer("first\nsecond");
        let prompt_lines = inline_composer_prompt_lines(&input_state, 40);

        assert_eq!(
            prompt_lines,
            vec!["› first".to_owned(), "  second".to_owned()]
        );
    }

    #[test]
    fn inline_composer_cursor_tracks_prompt_rows() {
        let input_state = WorkbenchInputState::default();
        assert_eq!(
            inline_composer_cursor(&input_state, 40),
            InlineComposerCursor { row: 1, column: 2 }
        );

        let mut input_state = WorkbenchInputState::default();
        input_state.set_buffer("first\nsecond");
        assert_eq!(
            inline_composer_cursor(&input_state, 40),
            InlineComposerCursor { row: 2, column: 8 }
        );
    }

    #[test]
    fn inline_composer_cursor_tracks_wrapped_prompt_rows() {
        let mut input_state = WorkbenchInputState::default();
        input_state.set_buffer("abcdefg");

        assert_eq!(
            inline_composer_prompt_lines(&input_state, 8),
            vec!["› abcdef".to_owned(), "  g".to_owned()]
        );
        assert_eq!(
            inline_composer_cursor(&input_state, 8),
            InlineComposerCursor { row: 2, column: 3 }
        );
    }

    #[test]
    fn codex_intro_card_width_matches_reference_shape() {
        let ui =
            WorkbenchLineInputUi::new("mock-chat", "/tmp/some/really/long/workspace/path", true);
        let width = codex_intro_card_width(&ui, 80);

        assert!((42..=56).contains(&width));
        assert!(codex_intro_box_line(">_ Ikaros", width).contains(">_ Ikaros"));
        assert_eq!(codex_intro_border('╭', '╮', width).chars().count(), width);
    }

    #[test]
    fn compact_middle_text_preserves_edges() {
        assert_eq!(
            compact_middle_text("/home/user/project/src/main.rs", 12),
            "/home…ain.rs"
        );
    }

    #[test]
    fn terminal_text_width_helpers_handle_cjk_input() {
        assert_eq!(terminal_display_width("你好"), 4);
        assert_eq!(fit_terminal_text("你好abc", 5), "你好a");
        assert_eq!(pad_terminal_text("你好", 6), "你好  ");
    }

    #[test]
    fn submitted_user_message_wraps_by_display_width() {
        let lines = submitted_user_message_lines("你好世界", 8);

        assert_eq!(lines, vec!["你好世".to_owned(), "界".to_owned()]);
    }
}
