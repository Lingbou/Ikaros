// SPDX-License-Identifier: GPL-3.0-only
//! Newline-gated markdown streaming helpers for terminal transcripts.

use crate::markdown::{TerminalMarkdownRenderer, is_table_delimiter_line, is_table_header_line};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MarkdownStreamCollector {
    buffer: String,
    committed_source_len: usize,
}

impl MarkdownStreamCollector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
        self.committed_source_len = 0;
    }

    pub fn push_delta(&mut self, delta: &str) {
        self.buffer.push_str(delta);
    }

    pub fn commit_complete_source(&mut self) -> Option<String> {
        let commit_end = self.buffer.rfind('\n').map(|index| index + 1)?;
        if commit_end <= self.committed_source_len {
            return None;
        }

        let source = self.buffer[self.committed_source_len..commit_end].to_owned();
        self.committed_source_len = commit_end;
        Some(source)
    }

    pub fn finalize_and_drain_source(&mut self) -> String {
        if self.committed_source_len >= self.buffer.len() {
            self.clear();
            return String::new();
        }

        let mut source = self.buffer[self.committed_source_len..].to_owned();
        if !source.ends_with('\n') {
            source.push('\n');
        }
        self.clear();
        source
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamFinish {
    pub lines: Vec<String>,
    pub raw_markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalStreamRenderer {
    collector: MarkdownStreamCollector,
    renderer: TerminalMarkdownRenderer,
    width: usize,
    raw_source: String,
    emitted_lines: usize,
}

impl TerminalStreamRenderer {
    pub fn new(width: usize) -> Self {
        Self {
            collector: MarkdownStreamCollector::new(),
            renderer: TerminalMarkdownRenderer::new(Some(width.max(1))),
            width: width.max(1),
            raw_source: String::new(),
            emitted_lines: 0,
        }
    }

    pub fn push_delta(&mut self, delta: &str) -> Vec<String> {
        if delta.is_empty() {
            return Vec::new();
        }

        self.collector.push_delta(delta);
        let Some(committed_source) = self.collector.commit_complete_source() else {
            return Vec::new();
        };

        self.raw_source.push_str(&committed_source);
        self.sync_stable_lines()
    }

    pub fn current_tail_lines(&self) -> Vec<String> {
        let rendered = self.render_source(&self.raw_source);
        let stable_len = self.stable_line_count(&rendered);
        if stable_len >= rendered.len() {
            Vec::new()
        } else {
            rendered[stable_len..].to_vec()
        }
    }

    pub fn finish(&mut self) -> Vec<String> {
        self.finish_with_source().lines
    }

    pub fn finish_with_source(&mut self) -> StreamFinish {
        let remaining_source = self.collector.finalize_and_drain_source();
        if !remaining_source.is_empty() {
            self.raw_source.push_str(&remaining_source);
        }

        if self.raw_source.is_empty() {
            self.reset();
            return StreamFinish {
                lines: Vec::new(),
                raw_markdown: String::new(),
            };
        }

        let rendered = self.render_source(&self.raw_source);
        let lines = if self.emitted_lines >= rendered.len() {
            Vec::new()
        } else {
            rendered[self.emitted_lines..].to_vec()
        };
        let raw_markdown = std::mem::take(&mut self.raw_source);
        self.reset();
        StreamFinish {
            lines,
            raw_markdown,
        }
    }

    pub fn raw_markdown(&self) -> &str {
        &self.raw_source
    }

    fn sync_stable_lines(&mut self) -> Vec<String> {
        let rendered = self.render_source(&self.raw_source);
        let stable_len = self.stable_line_count(&rendered);
        if self.emitted_lines >= stable_len {
            return Vec::new();
        }

        let lines = rendered[self.emitted_lines..stable_len].to_vec();
        self.emitted_lines = stable_len;
        lines
    }

    fn stable_line_count(&self, rendered: &[String]) -> usize {
        let Some(table_start) = table_holdback_start(&self.raw_source) else {
            return rendered.len();
        };
        self.render_source(&self.raw_source[..table_start.min(self.raw_source.len())])
            .len()
    }

    fn render_source(&self, source: &str) -> Vec<String> {
        self.renderer.render_lines(source)
    }

    fn reset(&mut self) {
        self.collector.clear();
        self.raw_source.clear();
        self.emitted_lines = 0;
    }
}

#[derive(Debug, Clone, Copy)]
struct PreviousTableLine {
    source_start: usize,
    is_header: bool,
}

#[derive(Debug, Clone, Copy)]
struct FenceState {
    marker: char,
    len: usize,
}

fn table_holdback_start(source: &str) -> Option<usize> {
    let mut offset = 0usize;
    let mut previous: Option<PreviousTableLine> = None;
    let mut pending_header_start: Option<usize> = None;
    let mut fence: Option<FenceState> = None;

    for raw_line in source.split_inclusive('\n') {
        let source_start = offset;
        let line = raw_line.trim_end_matches(['\r', '\n']);
        let inside_code_fence = fence.is_some();

        if !inside_code_fence {
            let is_header = is_table_header_line(line);
            let is_delimiter = is_table_delimiter_line(line);
            if let Some(previous) = previous
                && previous.is_header
                && is_delimiter
            {
                return Some(previous.source_start);
            }

            if !line.trim().is_empty() {
                pending_header_start = is_header.then_some(source_start);
            }
            previous = Some(PreviousTableLine {
                source_start,
                is_header,
            });
        } else if !line.trim().is_empty() {
            previous = None;
            pending_header_start = None;
        }

        advance_fence_state(&mut fence, line);
        offset = offset.saturating_add(raw_line.len());
    }

    pending_header_start
}

fn advance_fence_state(fence: &mut Option<FenceState>, line: &str) {
    let leading_spaces = line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == b' ')
        .count();
    if leading_spaces > 3 {
        return;
    }

    let trimmed = line[leading_spaces..].trim_start_matches('>');
    let Some((marker, len)) = parse_fence_marker(trimmed.trim_start()) else {
        return;
    };

    if let Some(open) = fence {
        if marker == open.marker && len >= open.len && trimmed.trim_start()[len..].trim().is_empty()
        {
            *fence = None;
        }
    } else {
        *fence = Some(FenceState { marker, len });
    }
}

fn parse_fence_marker(line: &str) -> Option<(char, usize)> {
    let marker = line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let len = line.chars().take_while(|ch| *ch == marker).count();
    (len >= 3).then_some((marker, len))
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    #[test]
    fn collector_commits_only_newline_terminated_source() {
        let mut collector = MarkdownStreamCollector::new();

        collector.push_delta("hello");
        assert_eq!(collector.commit_complete_source(), None);

        collector.push_delta(" world\npartial");
        assert_eq!(
            collector.commit_complete_source(),
            Some("hello world\n".to_owned())
        );
        assert_eq!(collector.commit_complete_source(), None);
        assert_eq!(collector.finalize_and_drain_source(), "partial\n");
    }

    #[test]
    fn stream_renderer_holds_table_until_finish() {
        let mut stream = TerminalStreamRenderer::new(80);

        assert_eq!(stream.push_delta("Intro\n"), vec!["Intro".to_owned()]);
        assert!(stream.push_delta("| 文件 | Status |\n").is_empty());
        assert!(stream.push_delta("| --- | --- |\n").is_empty());
        assert!(stream.push_delta("| src/lib.rs | changed |\n").is_empty());
        assert!(!stream.current_tail_lines().is_empty());

        let finish = stream.finish_with_source();

        assert_eq!(
            finish.raw_markdown,
            "Intro\n| 文件 | Status |\n| --- | --- |\n| src/lib.rs | changed |\n"
        );
        assert!(finish.lines.iter().any(|line| line.contains("文件")));
        assert!(
            finish
                .lines
                .iter()
                .any(|line| line.contains("src/lib.rs │ changed"))
        );
    }

    #[test]
    fn stream_renderer_finally_reflows_held_table() {
        let mut stream = TerminalStreamRenderer::new(80);

        assert!(stream.push_delta("| Name | 值 |\n").is_empty());
        assert!(stream.push_delta("| --- | --- |\n").is_empty());
        assert!(stream.push_delta("| x | 短 |\n").is_empty());
        assert!(stream.push_delta("| long-name | 很长 |\n").is_empty());

        let finish = stream.finish();

        assert_eq!(finish.len(), 4);
        assert_eq!(finish[0].width(), finish[1].width());
        assert_eq!(finish[1].width(), finish[2].width());
        assert!(finish[0].starts_with("Name"));
        assert!(finish[3].contains("long-name │ 很长"));
    }
}
