// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use crossterm::{
    cursor::{MoveTo, MoveToColumn, MoveUp},
    queue,
    style::Print,
    terminal::{Clear, ClearType},
};
use std::io::{self, Write};
pub(super) fn clear_inline_composer_rows(
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

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct WorkbenchLineEditorRenderState {
    pub(super) rows: u16,
    pub(super) viewport_anchored: bool,
    pub(super) cursor_row_from_top: u16,
    pub(super) composer_top: Option<u16>,
}

impl WorkbenchLineEditorRenderState {
    pub(super) fn clear_rows_before_history_insert(&mut self) -> Result<()> {
        clear_inline_composer_rows(self.rows, self.cursor_row_from_top, self.composer_top)?;
        self.rows = 0;
        self.viewport_anchored = false;
        self.cursor_row_from_top = 0;
        self.composer_top = None;
        Ok(())
    }
}
