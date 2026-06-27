// SPDX-License-Identifier: GPL-3.0-only

use ikaros_core::redact_secrets;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TerminalMarkdownRenderer {
    width: Option<usize>,
}

impl Default for TerminalMarkdownRenderer {
    fn default() -> Self {
        Self { width: Some(80) }
    }
}

impl TerminalMarkdownRenderer {
    pub fn new(width: Option<usize>) -> Self {
        Self { width }
    }

    pub fn width(&self) -> Option<usize> {
        self.width
    }

    pub fn render_lines(&self, input: &str) -> Vec<String> {
        render_terminal_markdown_lines(input, self.width)
    }

    pub fn render_text(&self, input: &str) -> String {
        self.render_lines(input).join("\n")
    }

    pub fn render(&self, input: &str, width: usize) -> Vec<String> {
        render_terminal_markdown_lines(input, Some(width.max(1)))
    }
}

pub fn render_terminal_markdown_lines(input: &str, width: Option<usize>) -> Vec<String> {
    let mut renderer = MarkdownLineRenderer::new(width);
    for raw_line in input.lines() {
        renderer.push_line(raw_line);
    }
    renderer.finish()
}

pub fn render_terminal_markdown(input: &str) -> String {
    TerminalMarkdownRenderer::default().render_text(input)
}

pub fn render_terminal_markdown_for_current_width(input: &str) -> String {
    TerminalMarkdownRenderer::new(Some(markdown_render_width())).render_text(input)
}

pub fn render_assistant_markdown_transcript(input: &str) -> String {
    prefix_assistant_transcript_lines(&render_terminal_markdown(input))
}

pub fn render_assistant_markdown_transcript_for_current_width(input: &str) -> String {
    prefix_assistant_transcript_lines(&render_terminal_markdown_for_current_width(input))
}

pub fn color_assistant_bullet_for_terminal(rendered: &str, terminal: bool) -> String {
    if !terminal {
        return rendered.to_owned();
    }
    rendered.replacen('*', "\x1b[32m*\x1b[0m", 1)
}

fn markdown_render_width() -> usize {
    crossterm::terminal::size()
        .map(|(width, _)| usize::from(width).saturating_sub(2).max(24))
        .unwrap_or(80)
}

fn prefix_assistant_transcript_lines(rendered: &str) -> String {
    let mut saw_content = false;
    rendered
        .lines()
        .map(|line| {
            if line.trim().is_empty() {
                return String::new();
            }
            if saw_content {
                format!("  {line}")
            } else {
                saw_content = true;
                format!(
                    "* {}",
                    line.strip_prefix("* ")
                        .map(str::to_owned)
                        .unwrap_or_else(|| line.to_owned())
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

struct MarkdownLineRenderer {
    width: Option<usize>,
    code_language: Option<String>,
    table_rows: Vec<Vec<String>>,
    rendered: Vec<String>,
}

impl MarkdownLineRenderer {
    fn new(width: Option<usize>) -> Self {
        Self {
            width,
            code_language: None,
            table_rows: Vec::new(),
            rendered: Vec::new(),
        }
    }

    fn push_line(&mut self, raw_line: &str) {
        let line = redact_secrets(raw_line);
        let trimmed = line.trim();
        if let Some(language) = trimmed.strip_prefix("```") {
            self.flush_table_rows();
            if self.code_language.take().is_some() {
                self.rendered.push("---".to_owned());
            } else {
                let label = if language.trim().is_empty() {
                    "code"
                } else {
                    language.trim()
                };
                self.code_language = Some(label.to_owned());
                self.rendered.push(format!("--- {label}"));
            }
            return;
        }

        if self.code_language.is_some() {
            self.rendered.push(format!("| {line}"));
            return;
        }

        if is_table_line(trimmed) {
            if !is_table_separator(trimmed) {
                self.table_rows.push(parse_table_row(trimmed));
            }
            return;
        }

        self.flush_table_rows();
        self.rendered
            .extend(render_markdown_line(&line, self.width));
    }

    fn finish(mut self) -> Vec<String> {
        if self.code_language.take().is_some() {
            self.rendered.push("---".to_owned());
        }
        self.flush_table_rows();
        collapse_consecutive_blank_lines(trim_trailing_blank_lines(self.rendered))
    }

    fn flush_table_rows(&mut self) {
        if self.table_rows.is_empty() {
            return;
        }
        let rows = std::mem::take(&mut self.table_rows);
        self.rendered.extend(render_table_rows(&rows, self.width));
    }
}

fn render_markdown_line(line: &str, width: Option<usize>) -> Vec<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return vec![String::new()];
    }
    let rendered = if is_horizontal_rule(trimmed) {
        "-".repeat(width.unwrap_or(80).max(24))
    } else if let Some(heading) = markdown_heading(trimmed) {
        heading
    } else if let Some(quote) = trimmed.strip_prefix("> ") {
        format!("> {}", render_inline_markdown_text(quote.trim()))
    } else if let Some(item) = markdown_unordered_item(trimmed) {
        format!("* {}", render_inline_markdown_text(item.trim()))
    } else if let Some((number, item)) = markdown_ordered_item(trimmed) {
        format!("{number}. {}", render_inline_markdown_text(item.trim()))
    } else {
        render_inline_markdown_text(line)
    };
    wrap_terminal_line(&rendered, width)
}

pub fn markdown_heading(trimmed: &str) -> Option<String> {
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if level == 0 || level > 6 || trimmed.as_bytes().get(level) != Some(&b' ') {
        return None;
    }
    let text = trimmed[level + 1..].trim();
    (!text.is_empty()).then(|| render_inline_markdown_text(text))
}

fn markdown_unordered_item(trimmed: &str) -> Option<&str> {
    ["- ", "* ", "+ "]
        .into_iter()
        .find_map(|prefix| trimmed.strip_prefix(prefix))
}

fn markdown_ordered_item(trimmed: &str) -> Option<(&str, &str)> {
    let dot = trimmed.find('.')?;
    let (number, rest) = trimmed.split_at(dot);
    let item = rest.strip_prefix(". ")?;
    (!number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit())).then_some((number, item))
}

fn is_horizontal_rule(trimmed: &str) -> bool {
    trimmed.len() >= 3
        && trimmed
            .chars()
            .all(|ch| ch == '-' || ch == '*' || ch == '_')
}

pub(crate) fn render_inline_markdown_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '`' => {}
            '*' | '_' if chars.peek().copied() == Some(ch) => {
                let _ = chars.next();
            }
            _ => out.push(ch),
        }
    }
    out
}

pub(crate) fn is_table_line(trimmed: &str) -> bool {
    if trimmed.is_empty() || !trimmed.contains('|') {
        return false;
    }
    let segments = trimmed
        .split('|')
        .filter(|cell| !cell.trim().is_empty())
        .count();
    segments >= 2
}

pub fn is_markdown_table_line(line: &str) -> bool {
    is_table_line(line.trim())
}

pub(crate) fn is_table_header_line(trimmed: &str) -> bool {
    is_table_line(trimmed) && !is_table_delimiter_line(trimmed)
}

pub(crate) fn is_table_delimiter_line(trimmed: &str) -> bool {
    is_table_separator(trimmed)
}

fn is_table_separator(trimmed: &str) -> bool {
    let content = trimmed.trim_matches('|').trim();
    !content.is_empty()
        && content
            .chars()
            .all(|ch| ch == '-' || ch == ':' || ch == '|' || ch.is_whitespace())
}

fn parse_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| render_inline_markdown_text(cell.trim()))
        .collect()
}

fn render_table_rows(rows: &[Vec<String>], width: Option<usize>) -> Vec<String> {
    if rows.is_empty() {
        return Vec::new();
    }
    let col_count = rows.iter().map(Vec::len).max().unwrap_or(0);
    if col_count == 0 {
        return Vec::new();
    }
    let mut widths = vec![0usize; col_count];
    for row in rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(cell.width());
        }
    }
    if let Some(max_width) = width {
        shrink_table_widths(&mut widths, max_width);
    }

    let mut out = Vec::new();
    for (row_idx, row) in rows.iter().enumerate() {
        out.push(render_table_row(row, &widths));
        if row_idx == 0 && rows.len() > 1 {
            out.push(render_table_separator(&widths));
        }
    }
    out
}

fn shrink_table_widths(widths: &mut [usize], max_width: usize) {
    if widths.is_empty() {
        return;
    }
    let separators = widths.len().saturating_sub(1) * 3;
    let available = max_width.saturating_sub(separators).max(widths.len());
    while widths.iter().sum::<usize>() > available {
        let Some((idx, width)) = widths.iter().enumerate().max_by_key(|(_, width)| *width) else {
            return;
        };
        if *width <= 8 {
            break;
        }
        widths[idx] = width.saturating_sub(1);
    }
}

fn render_table_row(row: &[String], widths: &[usize]) -> String {
    widths
        .iter()
        .enumerate()
        .map(|(idx, width)| {
            let cell = row.get(idx).map(String::as_str).unwrap_or("");
            pad_or_truncate(cell, *width)
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn render_table_separator(widths: &[usize]) -> String {
    widths
        .iter()
        .map(|width| "-".repeat((*width).max(3)))
        .collect::<Vec<_>>()
        .join("-+-")
}

fn pad_or_truncate(input: &str, width: usize) -> String {
    let input_width = input.width();
    if input_width == width {
        return input.to_owned();
    }
    if input_width < width {
        return format!("{input}{}", " ".repeat(width - input_width));
    }
    let ellipsis = "...";
    let target = width.saturating_sub(ellipsis.width());
    let mut out = String::new();
    let mut current = 0usize;
    for ch in input.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if current.saturating_add(ch_width) > target {
            break;
        }
        out.push(ch);
        current = current.saturating_add(ch_width);
    }
    out.push_str(ellipsis);
    out
}

fn wrap_terminal_line(line: &str, width: Option<usize>) -> Vec<String> {
    let Some(width) = width.map(|width| width.max(1)) else {
        return vec![line.to_owned()];
    };
    if line.width() <= width {
        return vec![line.to_owned()];
    }
    let continuation = continuation_prefix(line);
    let continuation_width = continuation.width();
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for ch in line.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if !current.is_empty() && current_width.saturating_add(ch_width) > width {
            rows.push(std::mem::take(&mut current));
            current_width = 0;
            if continuation_width < width {
                current.push_str(&continuation);
                current_width = continuation_width;
            }
        }
        current.push(ch);
        current_width = current_width.saturating_add(ch_width);
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

fn continuation_prefix(line: &str) -> String {
    if line.starts_with("* ") {
        "  ".to_owned()
    } else if line.starts_with("| ") {
        "| ".to_owned()
    } else {
        line.chars()
            .take_while(|ch| ch.is_whitespace())
            .collect::<String>()
    }
}

fn trim_trailing_blank_lines(mut lines: Vec<String>) -> Vec<String> {
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
    lines
}

fn collapse_consecutive_blank_lines(lines: Vec<String>) -> Vec<String> {
    let mut collapsed = Vec::with_capacity(lines.len());
    let mut previous_blank = false;
    for line in lines {
        let blank = line.trim().is_empty();
        if blank && previous_blank {
            continue;
        }
        previous_blank = blank;
        collapsed.push(line);
    }
    collapsed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renderer_cleans_common_markdown_markers() {
        let rendered = TerminalMarkdownRenderer::new(Some(80))
            .render_text("### **Summary**\n\n- Run `cargo check`\nplain **text**");

        assert_eq!(rendered, "Summary\n\n* Run cargo check\nplain text");
        assert!(!rendered.contains("###"));
        assert!(!rendered.contains("**"));
        assert!(!rendered.contains('`'));
    }

    #[test]
    fn renderer_collapses_consecutive_blank_lines() {
        let rendered =
            TerminalMarkdownRenderer::new(Some(80)).render_text("first\n\n\n\nsecond\n\n\nthird");

        assert_eq!(rendered, "first\n\nsecond\n\nthird");
        assert!(!rendered.contains("\n\n\n"));
    }

    #[test]
    fn renderer_formats_code_blocks() {
        let rendered =
            TerminalMarkdownRenderer::new(Some(80)).render_text("```rust\nfn main() {}\n```");

        assert_eq!(rendered, "--- rust\n| fn main() {}\n---");
    }

    #[test]
    fn renderer_aligns_tables() {
        let rendered = TerminalMarkdownRenderer::new(Some(80))
            .render_text("| Name | Status |\n| --- | --- |\n| Ikaros | Done |\n| TUI | Running |");

        assert!(rendered.contains("Name   | Status"));
        assert!(rendered.contains("Ikaros | Done"));
        assert!(rendered.contains("TUI    | Running"));
        assert!(!rendered.contains("| --- |"));
    }

    #[test]
    fn renderer_truncates_wide_tables_to_width() {
        let rendered = TerminalMarkdownRenderer::new(Some(24))
            .render_lines("| Column | Description |\n| --- | --- |\n| one | very very long text |");

        assert!(rendered.iter().all(|line| line.width() <= 24));
    }

    #[test]
    fn renderer_formats_code_diff_table_and_redacts_secrets() {
        let input = r#"Here is a patch:

### Summary

- item one

```diff
- old token=sk-secret-value
+ new value
```

| File | Status |
| --- | --- |
| src/lib.rs | changed |

```rust
fn main() {}
```
"#;

        let rendered = TerminalMarkdownRenderer::new(Some(80)).render_text(input);

        assert!(rendered.contains("Summary"));
        assert!(!rendered.contains("### Summary"));
        assert!(rendered.contains("* item one"));
        assert!(rendered.contains("--- diff"));
        assert!(rendered.contains("---"));
        assert!(rendered.contains("| - old [REDACTED_SECRET]"));
        assert!(rendered.contains("| + new value"));
        assert_rendered_table_contains(&rendered, &["File", "Status"]);
        assert_rendered_table_contains(&rendered, &["src/lib.rs", "changed"]);
        assert!(rendered.contains("--- rust"));
        assert!(rendered.contains("| fn main() {}"));
        assert!(!rendered.contains("[table]"));
        assert!(!rendered.contains("[code"));
        assert!(!rendered.contains("| --- |"));
        assert!(!rendered.contains("| File | Status |"));
        assert!(!rendered.contains("sk-secret-value"));
    }

    #[test]
    fn assistant_markdown_transcript_uses_codex_like_prefixes() {
        let rendered =
            render_assistant_markdown_transcript("### Summary\n\n- item one\n> quoted\nplain");

        assert_eq!(rendered, "* Summary\n\n  * item one\n  > quoted\n  plain");
        assert!(!rendered.contains("###"));
    }

    #[test]
    fn assistant_markdown_transcript_cleans_inline_markers_without_tree_artifacts() {
        let input = r#"### **Summary**

- Run `git status`
  - **nested** item
Plain **bold** and `inline code`.

```sh
echo hello
```
"#;

        let rendered = render_assistant_markdown_transcript(input);

        assert!(rendered.contains("* Summary"));
        assert!(rendered.contains("  * Run git status"));
        assert!(rendered.contains("  * nested item"));
        assert!(rendered.contains("  Plain bold and inline code."));
        assert!(rendered.contains("  --- sh"));
        assert!(rendered.contains("  | echo hello"));
        assert!(rendered.contains("  ---"));
        assert!(!rendered.contains("**"));
        assert!(!rendered.contains('`'));
        assert!(!rendered.contains("```"));
        assert!(!rendered.contains("- **nested**"));
    }

    #[test]
    fn assistant_markdown_transcript_formats_nonstream_markdown_like_live_output() {
        let input = r#"### Summary

> quoted

```rust
fn main() {}
```

| File | Status |
| --- | --- |
| src/lib.rs | changed |
"#;

        let rendered = render_assistant_markdown_transcript(input);

        assert!(rendered.contains("* Summary"));
        assert!(rendered.contains("  > quoted"));
        assert!(rendered.contains("  --- rust"));
        assert!(rendered.contains("  | fn main() {}"));
        assert!(rendered.contains("  ---"));
        assert_rendered_table_contains(&rendered, &["File", "Status"]);
        assert_rendered_table_contains(&rendered, &["src/lib.rs", "changed"]);
        assert!(!rendered.contains("###"));
        assert!(!rendered.contains("```"));
    }

    #[test]
    fn assistant_bullet_color_is_terminal_only() {
        let rendered = "* Summary\n  * item";

        assert_eq!(
            color_assistant_bullet_for_terminal(rendered, true),
            "\x1b[32m*\x1b[0m Summary\n  * item"
        );
        assert_eq!(
            color_assistant_bullet_for_terminal(rendered, false),
            rendered
        );
    }

    fn assert_rendered_table_contains(rendered: &str, expected_cells: &[&str]) {
        let rows = rendered_table_rows(rendered);
        assert!(
            rows.iter().any(|row| row.as_slice() == expected_cells),
            "missing rendered table row {expected_cells:?} in:\n{rendered}"
        );
    }

    fn rendered_table_rows(rendered: &str) -> Vec<Vec<&str>> {
        rendered
            .lines()
            .filter_map(|line| {
                let cells = line
                    .split(['|', '\u{2502}'])
                    .map(str::trim)
                    .filter(|cell| !cell.is_empty())
                    .collect::<Vec<_>>();
                (cells.len() >= 2).then_some(cells)
            })
            .collect()
    }
}
