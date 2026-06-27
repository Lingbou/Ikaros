// SPDX-License-Identifier: GPL-3.0-only

use super::{
    display::{terminal_width, wrap_display_line},
    inline_composer::{queue_codex_input_cell_padding_line, queue_codex_input_cell_text_line},
};
use crate::text::fit_terminal_text;
use crate::{ResetScrollRegion, SetScrollRegion, terminal_inline, terminal_message};
use anyhow::Result;
use crossterm::{
    cursor::MoveTo,
    queue,
    style::{Color, Print},
};
use std::io::{self, IsTerminal, Write};
pub(super) fn insert_submitted_user_message(input: &str) -> Result<()> {
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

pub(super) fn print_submitted_user_message_at_cursor(input: &str) -> Result<()> {
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

pub(super) fn queue_submitted_user_message_history_insert(
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

pub(super) fn queue_submitted_user_message_cell_line(
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
                [(Color::Cyan, "* "), (Color::White, text.as_str())],
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

pub(super) fn submitted_user_message_lines(input: &str, width: usize) -> Vec<String> {
    let wrap_width = width.saturating_sub(2).max(1);
    let mut lines = Vec::new();
    let input = terminal_message(input)
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
pub(super) struct SubmittedUserMessageCellLine {
    pub(super) text: String,
    pub(super) kind: SubmittedUserMessageCellLineKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SubmittedUserMessageCellLineKind {
    Spacer,
    First,
    Continuation,
}

pub(super) fn submitted_user_message_cell_lines(
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
