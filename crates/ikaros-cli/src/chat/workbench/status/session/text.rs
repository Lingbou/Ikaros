// SPDX-License-Identifier: GPL-3.0-only

use super::super::super::terminal_inline;

pub(super) fn truncate_session_text(input: &str, max_chars: usize) -> String {
    let input = terminal_inline(input).replace('\n', " ");
    if input.chars().count() <= max_chars {
        return input;
    }
    let mut output = input
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    output.push_str("...");
    output
}
