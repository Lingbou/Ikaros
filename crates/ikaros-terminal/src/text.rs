// SPDX-License-Identifier: GPL-3.0-only

use crate::terminal_inline;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(crate) fn fit_terminal_text(input: &str, width: usize) -> String {
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

pub(crate) fn pad_terminal_text(input: &str, width: usize) -> String {
    let width = width.max(1);
    let mut output = fit_terminal_text(input, width);
    let count = terminal_display_width(&output);
    if count < width {
        output.push_str(&" ".repeat(width - count));
    }
    output
}

pub(crate) fn terminal_display_width(input: &str) -> usize {
    UnicodeWidthStr::width(input)
}

pub(crate) fn terminal_char_width(ch: char) -> usize {
    UnicodeWidthChar::width(ch).unwrap_or(0)
}

pub(crate) fn take_display_prefix(input: &str, max_width: usize) -> String {
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

pub(crate) fn take_display_suffix(input: &str, max_width: usize) -> String {
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

pub(crate) fn strip_ansi_sequences(input: &str) -> String {
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

pub(crate) fn collect_ansi_sequence(
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
