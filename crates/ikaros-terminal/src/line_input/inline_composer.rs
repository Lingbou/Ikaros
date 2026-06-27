// SPDX-License-Identifier: GPL-3.0-only

use super::{
    INLINE_COMPOSER_HINT_LIMIT,
    display::{
        compact_path_label, inline_viewport_spacer_rows, terminal_dimensions, wrap_display_line,
    },
    popup::inline_completion_popup_lines,
    render_state::WorkbenchLineEditorRenderState,
    ui::WorkbenchLineInputUi,
};
use crate::text::{fit_terminal_text, terminal_display_width};
use crate::{WorkbenchInputState, terminal_inline};
use anyhow::Result;
use crossterm::{
    cursor::{MoveTo, MoveToColumn, MoveUp},
    queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use std::io::{self, Write};
pub(super) fn render_workbench_inline_composer(
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

pub(super) fn inline_composer_repaint_top(
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

pub(super) fn queue_inline_composer_cursor(
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

pub(super) fn queue_inline_composer_line(
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
            let rest = line.text.strip_prefix("* ").unwrap_or(&line.text);
            let rest = fit_terminal_text(rest, width.saturating_sub(2));
            queue_codex_input_cell_text_line(
                stdout,
                [(Color::White, "* "), (Color::Grey, rest.as_str())],
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

pub(super) fn queue_inline_footer(stdout: &mut impl Write, text: &str, width: usize) -> Result<()> {
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

pub(super) fn queue_codex_input_cell_padding_line(stdout: &mut impl Write) -> Result<()> {
    queue!(
        stdout,
        SetBackgroundColor(Color::DarkGrey),
        Clear(ClearType::UntilNewLine),
        ResetColor
    )?;
    Ok(())
}

pub(super) fn queue_codex_input_cell_text_line<'a>(
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct InlineComposerLine {
    pub(super) text: String,
    pub(super) style: InlineComposerLineStyle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InlineComposerLineStyle {
    Hint,
    SelectedHint,
    Prompt,
    Placeholder,
    Footer,
}

pub(super) fn inline_composer_lines(
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

pub(super) fn inline_composer_prompt_lines(
    input_state: &WorkbenchInputState,
    width: usize,
) -> Vec<String> {
    if input_state.buffer().is_empty() {
        return vec![fit_terminal_text("* Ask Ikaros to do anything", width)];
    }
    let content_width = width.saturating_sub(2).max(1);
    let mut rows = Vec::new();
    for raw_line in input_state.buffer().split('\n') {
        let wrapped = wrap_display_line(raw_line, content_width);
        for segment in wrapped {
            let prefix = if rows.is_empty() { "* " } else { "  " };
            rows.push(fit_terminal_text(&format!("{prefix}{segment}"), width));
        }
    }
    rows
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct InlineComposerCursor {
    pub(super) row: u16,
    pub(super) column: u16,
}

pub(super) fn inline_composer_cursor(
    input_state: &WorkbenchInputState,
    width: usize,
) -> InlineComposerCursor {
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
