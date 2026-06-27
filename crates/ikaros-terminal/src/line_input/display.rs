// SPDX-License-Identifier: GPL-3.0-only

use crate::terminal_inline;
use crate::text::{
    take_display_prefix, take_display_suffix, terminal_char_width, terminal_display_width,
};
use crossterm::terminal::size as terminal_size;
pub(super) fn wrap_display_line(line: &str, width: usize) -> Vec<String> {
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

pub(super) fn compact_path_label(path: &str, max_width: usize) -> String {
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

pub(super) fn compact_middle_text(input: &str, max_width: usize) -> String {
    if terminal_display_width(input) <= max_width {
        return input.to_owned();
    }
    if max_width <= 1 {
        return ".".to_owned();
    }
    if max_width <= 4 {
        return take_display_prefix(input, max_width);
    }
    let marker = "...";
    let keep = max_width.saturating_sub(terminal_display_width(marker));
    let head = keep / 2;
    let tail = keep.saturating_sub(head);
    let prefix = take_display_prefix(input, head);
    let suffix = take_display_suffix(input, tail);
    format!("{prefix}{marker}{suffix}")
}

pub(super) fn terminal_width() -> usize {
    terminal_dimensions().0
}

pub(super) fn terminal_dimensions() -> (usize, u16) {
    terminal_size()
        .map(|(width, height)| (usize::from(width).max(20), height.max(1)))
        .unwrap_or((80, 24))
}

pub(super) fn inline_viewport_spacer_rows(
    _cursor_y: u16,
    _terminal_height: u16,
    _composer_rows: u16,
) -> u16 {
    0
}
