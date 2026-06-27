// SPDX-License-Identifier: GPL-3.0-only

use crate::{TerminalMarkdownRenderer, ToolActivity, render_tool_activity};
use ikaros_core::redact_secrets;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub trait TranscriptCell: std::fmt::Debug + Send + Sync {
    fn render_lines(&self, width: usize) -> Vec<String>;
    fn raw_lines(&self) -> Vec<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserCell {
    message: String,
}

impl UserCell {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl TranscriptCell for UserCell {
    fn render_lines(&self, width: usize) -> Vec<String> {
        let content_width = width.saturating_sub(2).max(1);
        let mut rows = vec![String::new()];
        let message = redact_secrets(&self.message)
            .trim_end_matches(['\r', '\n'])
            .to_owned();
        if message.is_empty() {
            rows.push("› ".to_owned());
        } else {
            for (line_idx, raw_line) in message.split('\n').enumerate() {
                let wrapped = wrap_display_line(raw_line, content_width);
                for (wrap_idx, line) in wrapped.into_iter().enumerate() {
                    let prefix = if line_idx == 0 && wrap_idx == 0 {
                        "› "
                    } else {
                        "  "
                    };
                    rows.push(format!("{prefix}{line}"));
                }
            }
        }
        rows.push(String::new());
        rows
    }

    fn raw_lines(&self) -> Vec<String> {
        redact_secrets(&self.message)
            .trim_end_matches(['\r', '\n'])
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistantMarkdownCell {
    markdown_source: String,
}

impl AssistantMarkdownCell {
    pub fn new(markdown_source: impl Into<String>) -> Self {
        Self {
            markdown_source: markdown_source.into(),
        }
    }

    pub fn markdown_source(&self) -> &str {
        &self.markdown_source
    }
}

impl TranscriptCell for AssistantMarkdownCell {
    fn render_lines(&self, width: usize) -> Vec<String> {
        prefix_assistant_lines(
            TerminalMarkdownRenderer::new(Some(width.saturating_sub(2).max(1)))
                .render_lines(&self.markdown_source),
        )
    }

    fn raw_lines(&self) -> Vec<String> {
        redact_secrets(&self.markdown_source)
            .lines()
            .map(str::to_owned)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamingTailCell {
    lines: Vec<String>,
    first_line: bool,
}

impl StreamingTailCell {
    pub fn new(lines: Vec<String>, first_line: bool) -> Self {
        Self { lines, first_line }
    }
}

impl TranscriptCell for StreamingTailCell {
    fn render_lines(&self, _width: usize) -> Vec<String> {
        let mut saw_first = !self.first_line;
        self.lines
            .iter()
            .map(|line| {
                if line.trim().is_empty() {
                    return String::new();
                }
                if saw_first {
                    format!("  {line}")
                } else {
                    saw_first = true;
                    format!("• {line}")
                }
            })
            .collect()
    }

    fn raw_lines(&self) -> Vec<String> {
        self.lines.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolActivityCell {
    activity: ToolActivity,
}

impl ToolActivityCell {
    pub fn new(activity: ToolActivity) -> Self {
        Self { activity }
    }

    pub fn activity(&self) -> &ToolActivity {
        &self.activity
    }
}

impl TranscriptCell for ToolActivityCell {
    fn render_lines(&self, width: usize) -> Vec<String> {
        render_tool_activity(&self.activity)
            .into_iter()
            .flat_map(|line| wrap_display_line(&line, width.max(1)))
            .collect()
    }

    fn raw_lines(&self) -> Vec<String> {
        render_tool_activity(&self.activity)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeparatorCell;

impl SeparatorCell {
    pub fn new() -> Self {
        Self
    }
}

impl TranscriptCell for SeparatorCell {
    fn render_lines(&self, width: usize) -> Vec<String> {
        vec![String::new(), "─".repeat(width.max(24)), String::new()]
    }

    fn raw_lines(&self) -> Vec<String> {
        vec![String::new(), "─".repeat(80), String::new()]
    }
}

fn prefix_assistant_lines(lines: Vec<String>) -> Vec<String> {
    let mut saw_content = false;
    lines
        .into_iter()
        .map(|line| {
            if line.trim().is_empty() {
                return String::new();
            }
            if saw_content {
                format!("  {line}")
            } else {
                saw_content = true;
                format!("• {line}")
            }
        })
        .collect()
}

fn wrap_display_line(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if line.width() <= width {
        return vec![line.to_owned()];
    }
    let mut rows = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for ch in line.chars() {
        let ch_width = ch.width().unwrap_or(0);
        if !current.is_empty() && current_width.saturating_add(ch_width) > width {
            rows.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(ch);
        current_width = current_width.saturating_add(ch_width);
    }
    if !current.is_empty() {
        rows.push(current);
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ToolActivityStatus, TranscriptCell};

    #[test]
    fn user_cell_wraps_and_prefixes_message() {
        let cell = UserCell::new("你好 Ikaros");

        assert_eq!(
            cell.render_lines(12),
            vec![
                "".to_owned(),
                "› 你好 Ikaro".to_owned(),
                "  s".to_owned(),
                "".to_owned()
            ]
        );
    }

    #[test]
    fn assistant_cell_renders_from_markdown_source() {
        let cell = AssistantMarkdownCell::new("### Summary\n\n- item");

        assert_eq!(
            cell.render_lines(80),
            vec!["• Summary".to_owned(), String::new(), "  • item".to_owned()]
        );
    }

    #[test]
    fn streaming_tail_cell_marks_first_line_once() {
        let cell = StreamingTailCell::new(vec!["hello".into(), "world".into()], true);

        assert_eq!(cell.render_lines(80), vec!["• hello", "  world"]);
    }

    #[test]
    fn tool_activity_cell_uses_activity_renderer() {
        let cell = ToolActivityCell::new(
            ToolActivity::new("read_file", ToolActivityStatus::Completed).with_detail("Read a.rs"),
        );

        assert_eq!(cell.render_lines(80), vec!["• Explored", "  └ Read a.rs"]);
    }
}
