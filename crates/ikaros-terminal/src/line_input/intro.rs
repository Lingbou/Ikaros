// SPDX-License-Identifier: GPL-3.0-only

use super::{
    display::{compact_path_label, terminal_width},
    ui::WorkbenchLineInputUi,
};
use crate::text::{fit_terminal_text, pad_terminal_text, terminal_display_width};
use anyhow::Result;
use crossterm::{
    queue,
    style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor},
};
use std::io::{self, Write};
pub(super) fn render_workbench_inline_intro(ui: &WorkbenchLineInputUi) -> Result<()> {
    let width = terminal_width();
    let mut stdout = io::stdout();
    queue_workbench_inline_intro(&mut stdout, ui, width)?;
    stdout.flush()?;
    Ok(())
}

pub(super) fn queue_workbench_inline_intro(
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
        Print(codex_intro_border('+', '+', card_width)),
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
        Print(codex_intro_border('+', '+', card_width)),
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
        Print("|"),
        ResetColor,
        Print(" "),
        Print(padded),
        Print(" "),
        SetForegroundColor(Color::DarkGrey),
        Print("|"),
        ResetColor,
        Print("\r\n")
    )?;
    Ok(())
}

pub(super) fn codex_intro_card_width(ui: &WorkbenchLineInputUi, terminal_width: usize) -> usize {
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

pub(super) fn codex_intro_border(left: char, right: char, width: usize) -> String {
    let inner = width.saturating_sub(2);
    format!("{left}{}{right}", "-".repeat(inner))
}
