// SPDX-License-Identifier: GPL-3.0-only

use anyhow::Result;
use std::io::{self, Write};

pub const BRACKETED_PASTE_START: &str = "\u{1b}[200~";
const BRACKETED_PASTE_END: &str = "\u{1b}[201~";
pub const MULTILINE_TERMINATOR: &str = ".";

pub fn read_multiline_message() -> Result<Option<String>> {
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
        if trimmed == MULTILINE_TERMINATOR {
            return Ok(Some(message));
        }
        message.push_str(trimmed);
        message.push('\n');
    }
}

pub fn read_bracketed_paste_message(first_line: &str) -> Result<Option<String>> {
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
