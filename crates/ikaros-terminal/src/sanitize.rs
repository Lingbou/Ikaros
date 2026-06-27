// SPDX-License-Identifier: GPL-3.0-only

pub fn terminal_inline(input: &str) -> String {
    let stripped = strip_terminal_control_sequences(input);
    ikaros_core::redact_secrets(&strip_bare_sgr_mouse_sequences(&stripped))
        .chars()
        .map(|ch| if ch.is_control() { '_' } else { ch })
        .collect()
}

pub fn terminal_message(input: &str) -> String {
    let stripped = strip_terminal_control_sequences(input);
    ikaros_core::redact_secrets(&strip_bare_sgr_mouse_sequences(&stripped))
        .chars()
        .filter_map(|ch| match ch {
            '\n' => Some('\n'),
            '\r' => None,
            ch if ch.is_control() => Some('_'),
            ch => Some(ch),
        })
        .collect()
}

fn strip_terminal_control_sequences(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '\u{1b}' {
            output.push(ch);
            continue;
        }
        match chars.peek().copied() {
            Some('[') => {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            }
            Some(']') => {
                chars.next();
                skip_until_string_terminator(&mut chars);
            }
            Some('P' | '_' | '^') => {
                chars.next();
                skip_until_string_terminator(&mut chars);
            }
            Some(_) => {
                chars.next();
            }
            None => {}
        }
    }
    output
}

fn strip_bare_sgr_mouse_sequences(input: &str) -> String {
    let chars = input.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(input.len());
    let mut index = 0usize;
    'outer: while index < chars.len() {
        if chars[index] == '['
            && chars.get(index + 1) == Some(&'[')
            && chars.get(index + 2) == Some(&'<')
        {
            let mut cursor = index + 3;
            let mut has_digit = false;
            let mut semicolons = 0usize;
            while cursor < chars.len() {
                match chars[cursor] {
                    '0'..='9' => {
                        has_digit = true;
                        cursor += 1;
                    }
                    ';' => {
                        semicolons += 1;
                        cursor += 1;
                    }
                    'M' | 'm' if has_digit && semicolons >= 2 => {
                        index = cursor + 1;
                        continue 'outer;
                    }
                    _ => break,
                }
            }
        }
        if chars[index] == '[' && chars.get(index + 1) == Some(&'<') {
            let mut cursor = index + 2;
            let mut has_digit = false;
            let mut semicolons = 0usize;
            while cursor < chars.len() {
                match chars[cursor] {
                    '0'..='9' => {
                        has_digit = true;
                        cursor += 1;
                    }
                    ';' => {
                        semicolons += 1;
                        cursor += 1;
                    }
                    'M' | 'm' if has_digit && semicolons >= 2 => {
                        index = cursor + 1;
                        continue 'outer;
                    }
                    _ => break,
                }
            }
            if cursor == chars.len() && (has_digit || semicolons > 0) {
                break;
            }
        }
        output.push(chars[index]);
        index += 1;
    }
    output
}

fn skip_until_string_terminator(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(ch) = chars.next() {
        if ch == '\u{7}' {
            break;
        }
        if ch == '\u{1b}' && chars.peek() == Some(&'\\') {
            chars.next();
            break;
        }
    }
}
