// SPDX-License-Identifier: GPL-3.0-only

use crate::{sanitize::terminal_message, streaming::TerminalStreamRenderer};
use crossterm::terminal::size as terminal_size;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTextDeltaState {
    pub started: bool,
    pub activity_since_last_text: bool,
    pub has_pending_source: bool,
    pub last_delta_ended_with_newline: bool,
    pub pending_source: String,
    pub renderer: TerminalStreamRenderer,
}

impl Default for AgentTextDeltaState {
    fn default() -> Self {
        Self {
            started: false,
            activity_since_last_text: false,
            has_pending_source: false,
            last_delta_ended_with_newline: false,
            pending_source: String::new(),
            renderer: TerminalStreamRenderer::new(assistant_stream_body_width()),
        }
    }
}

pub fn terminal_width() -> usize {
    terminal_size()
        .map(|(width, _)| usize::from(width).max(24))
        .unwrap_or(80)
}

fn activity_answer_separator_line(width: usize) -> String {
    "─".repeat(width.max(24))
}

fn assistant_stream_body_width() -> usize {
    terminal_width().saturating_sub(2).max(1)
}

pub fn format_agent_text_delta(text: &str, state: &mut AgentTextDeltaState) -> String {
    let text = terminal_message(text);
    if text.is_empty() {
        return String::new();
    }

    state.has_pending_source = true;
    state.last_delta_ended_with_newline = text.ends_with('\n');
    state.pending_source.push_str(&text);
    let stable_source = drain_stable_agent_stream_source(state);
    if stable_source.is_empty() {
        return String::new();
    }
    let lines = state.renderer.push_delta(&stable_source);
    render_agent_stream_lines(&lines, state, true)
}

pub fn finish_agent_text_delta(state: &mut AgentTextDeltaState) -> String {
    if !state.started && !state.has_pending_source {
        return String::new();
    }

    let terminate_last_line = state.last_delta_ended_with_newline;
    let pending_source = std::mem::take(&mut state.pending_source);
    let mut lines = if pending_source.is_empty() {
        Vec::new()
    } else {
        state.renderer.push_delta(&pending_source)
    };
    let finish = state.renderer.finish_with_source();
    lines.extend(finish.lines);
    let _raw_markdown_source = finish.raw_markdown;
    let output = render_agent_stream_lines(&lines, state, terminate_last_line);
    state.started = false;
    state.has_pending_source = false;
    state.last_delta_ended_with_newline = false;
    output
}

fn render_agent_stream_lines(
    lines: &[String],
    state: &mut AgentTextDeltaState,
    terminate_last_line: bool,
) -> String {
    if lines.is_empty() {
        return String::new();
    }

    let mut output = String::new();
    let mut first_stream_line = !state.started;
    if first_stream_line {
        if state.activity_since_last_text {
            output.push('\n');
            output.push_str(&activity_answer_separator_line(terminal_width()));
            output.push_str("\n\n");
            state.activity_since_last_text = false;
        }
        state.started = true;
    }

    for (index, line) in lines.iter().enumerate() {
        if line.is_empty() {
            if first_stream_line {
                output.push_str("* ");
            }
        } else if first_stream_line {
            output.push_str("* ");
        } else {
            output.push_str("  ");
        }
        output.push_str(line);

        if index + 1 < lines.len() || terminate_last_line {
            output.push('\n');
        }
        first_stream_line = false;
    }

    output
}

fn drain_stable_agent_stream_source(state: &mut AgentTextDeltaState) -> String {
    let stable_len = stable_agent_stream_source_len(&state.pending_source);
    if stable_len == 0 {
        return String::new();
    }
    state.pending_source.drain(..stable_len).collect()
}

fn stable_agent_stream_source_len(source: &str) -> usize {
    let mut offset = 0usize;
    let mut stable_end = 0usize;
    let mut fence: Option<AgentStreamFenceState> = None;

    for raw_line in source.split_inclusive('\n') {
        if !raw_line.ends_with('\n') {
            break;
        }

        let line = raw_line.trim_end_matches(['\r', '\n']);
        advance_agent_stream_fence(&mut fence, line);
        offset = offset.saturating_add(raw_line.len());
        if fence.is_none() {
            stable_end = offset;
        }
    }

    stable_end
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AgentStreamFenceState {
    marker: char,
    len: usize,
}

fn advance_agent_stream_fence(fence: &mut Option<AgentStreamFenceState>, line: &str) {
    let leading_spaces = line
        .as_bytes()
        .iter()
        .take_while(|byte| **byte == b' ')
        .count();
    if leading_spaces > 3 {
        return;
    }

    let trimmed = line[leading_spaces..].trim_start_matches('>');
    let Some((marker, len)) = parse_agent_stream_fence_marker(trimmed.trim_start()) else {
        return;
    };

    if let Some(open) = fence {
        let closes_open_fence = marker == open.marker
            && len >= open.len
            && trimmed.trim_start()[len..].trim().is_empty();
        if closes_open_fence {
            *fence = None;
        }
    } else {
        *fence = Some(AgentStreamFenceState { marker, len });
    }
}

fn parse_agent_stream_fence_marker(line: &str) -> Option<(char, usize)> {
    let marker = line.chars().next()?;
    if !matches!(marker, '`' | '~') {
        return None;
    }
    let len = line.chars().take_while(|ch| *ch == marker).count();
    (len >= 3).then_some((marker, len))
}

#[cfg(test)]
mod tests {
    use crate::{AgentTextDeltaState, finish_agent_text_delta, format_agent_text_delta};

    #[test]
    fn agent_text_delta_uses_terminal_owned_api() {
        let mut state = AgentTextDeltaState::default();

        assert_eq!(format_agent_text_delta("hello", &mut state), "");
        assert_eq!(
            format_agent_text_delta(" world\n", &mut state),
            "* hello world\n"
        );
        assert_eq!(format_agent_text_delta("next line", &mut state), "");
        assert_eq!(finish_agent_text_delta(&mut state), "  next line");
    }
}
