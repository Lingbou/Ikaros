// SPDX-License-Identifier: GPL-3.0-only

use super::fit;
use crate::{Buffer, terminal_inline};

pub(crate) fn buffer_snapshot(buffer: &Buffer) -> String {
    let width = buffer.area.width as usize;
    buffer
        .content()
        .chunks(width)
        .map(|cells| {
            cells
                .iter()
                .filter(|cell| !cell.skip)
                .map(|cell| cell.symbol())
                .collect::<String>()
                .trim_end()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn border_title(title: &str, width: usize) -> String {
    let title = format!(" {} ", terminal_inline(title).to_ascii_uppercase());
    if title.chars().count() + 2 >= width {
        return framed_line(&title, width);
    }
    let side = width.saturating_sub(title.chars().count() + 2);
    let left = side / 2;
    let right = side.saturating_sub(left);
    format!("+{}{}{}+", "-".repeat(left), title, "-".repeat(right))
}

pub(crate) fn separator(width: usize) -> String {
    format!("+{}+", "-".repeat(width.saturating_sub(2)))
}

pub(crate) fn framed_line(text: &str, width: usize) -> String {
    let inside = fit(terminal_inline(text), width.saturating_sub(2));
    format!("|{inside}|")
}

pub(crate) fn three_column_row(
    left: impl AsRef<str>,
    main: impl AsRef<str>,
    side: impl AsRef<str>,
    width: usize,
) -> String {
    let (left_width, main_width, side_width) = column_widths(width);
    format!(
        "|{}|{}|{}|",
        fit(terminal_inline(left.as_ref()), left_width),
        fit(terminal_inline(main.as_ref()), main_width),
        fit(terminal_inline(side.as_ref()), side_width)
    )
}

pub(crate) fn column_widths(width: usize) -> (usize, usize, usize) {
    let inner = width.saturating_sub(4).max(36);
    let left = (inner / 4).max(10);
    let side = (inner / 3).max(12);
    let main = inner.saturating_sub(left + side).max(10);
    (left, main, side)
}
