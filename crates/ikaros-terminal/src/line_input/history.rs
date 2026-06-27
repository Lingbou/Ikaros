// SPDX-License-Identifier: GPL-3.0-only

use super::display::terminal_width;
use crate::text::{
    collect_ansi_sequence, strip_ansi_sequences, terminal_char_width, terminal_display_width,
};
use crate::{ResetScrollRegion, SetScrollRegion};
use anyhow::Result;
use crossterm::{
    cursor::MoveTo,
    queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use std::io::{self, IsTerminal, Write};
fn ikaros_accent_color() -> Color {
    Color::Rgb {
        r: 255,
        g: 154,
        b: 252,
    }
}

pub fn print_inline_turn_separator() {
    print!("{}", inline_turn_separator_text(terminal_width()));
}

pub fn print_inline_turn_worked_separator(elapsed_ms: u128) -> Result<()> {
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

pub fn clear_visible_terminal() -> Result<()> {
    if !io::stdout().is_terminal() {
        return Ok(());
    }
    let mut stdout = io::stdout();
    queue!(stdout, Clear(ClearType::All), MoveTo(0, 0))?;
    stdout.flush()?;
    Ok(())
}

pub(super) fn inline_turn_separator_text(width: usize) -> String {
    format!("\n{}\n\n", "─".repeat(width.max(24)))
}

pub(super) fn inline_turn_worked_separator_text(width: usize, elapsed_ms: u128) -> String {
    format!(
        "\n{}\n\n",
        inline_labeled_separator(width, &inline_turn_worked_label(elapsed_ms))
    )
}

pub(super) fn inline_turn_worked_label(elapsed_ms: u128) -> String {
    format!(" Worked for {} ", format_elapsed_duration(elapsed_ms))
}

pub(super) fn inline_labeled_separator(width: usize, label: &str) -> String {
    let width = width.max(24);
    let label_width = terminal_display_width(label);
    if label_width >= width.saturating_sub(2) {
        return "─".repeat(width);
    }
    let left = 2usize;
    let right = width.saturating_sub(left + label_width);
    format!("{}{}{}", "─".repeat(left), label, "─".repeat(right))
}

pub(super) fn format_elapsed_duration(elapsed_ms: u128) -> String {
    let mut seconds = elapsed_ms / 1000;
    let minutes = seconds / 60;
    seconds %= 60;
    if minutes > 0 {
        format!("{minutes}m {seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

pub fn print_inline_history_text(text: &str) -> Result<()> {
    let lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
    print_inline_history_lines(&lines)
}

pub fn print_inline_history_lines(lines: &[String]) -> Result<()> {
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

pub(super) fn wrap_inline_history_lines(lines: &[String], width: usize) -> Vec<String> {
    lines
        .iter()
        .flat_map(|line| wrap_inline_history_line(line, width))
        .collect()
}

pub(super) fn wrap_inline_history_line(line: &str, width: usize) -> Vec<String> {
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

pub(super) fn inline_history_continuation_prefix(line: &str) -> String {
    let visible = strip_ansi_sequences(line);
    if visible.starts_with("* ") || visible.starts_with("* ") {
        return "  ".to_owned();
    }
    visible
        .chars()
        .take_while(|ch| ch.is_whitespace())
        .collect()
}

pub(super) fn ansi_stripped_display_width(input: &str) -> usize {
    terminal_display_width(&strip_ansi_sequences(input))
}

pub(super) fn queue_inline_history_lines_insert(
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
