// SPDX-License-Identifier: GPL-3.0-only

use crate::{is_markdown_table_line, markdown_heading, render_terminal_markdown, terminal_inline};

pub(crate) fn wrap(input: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    for raw_line in input.lines() {
        let line = terminal_inline(raw_line);
        if line.trim().is_empty() {
            lines.push(String::new());
            continue;
        }
        lines.extend(wrap_single_line(&line, width));
    }
    if input.is_empty() {
        lines.push("none".into());
    }
    while lines.last().is_some_and(|line| line.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        lines.push("none".into());
    }
    lines
}

pub(crate) fn wrap_markdown_detail(input: &str, width: usize) -> Vec<String> {
    wrap(&render_terminal_markdown(input), width)
}

pub(crate) fn render_cell_detail_summary(input: &str) -> String {
    render_terminal_markdown(input)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" / ")
}

pub(crate) fn markdown_feature_json(raw: &str, rendered: &str) -> serde_json::Value {
    let has_code = rendered.contains("[code") || rendered.contains("[diff]");
    let has_diff = rendered.contains("[diff]")
        || raw.lines().any(|line| {
            let trimmed = line.trim_start();
            trimmed.starts_with("+ ") || trimmed.starts_with("- ") || trimmed.starts_with("@@")
        });
    let has_table = rendered.contains("[table]") || raw.lines().any(is_markdown_table_line);
    let has_heading = raw
        .lines()
        .any(|line| markdown_heading(line.trim()).is_some());
    let has_list = raw.lines().any(|line| {
        let trimmed = line.trim();
        markdown_unordered_item_like(trimmed) || markdown_ordered_item_like(trimmed)
    });
    serde_json::json!({
        "has_code": has_code,
        "has_diff": has_diff,
        "has_table": has_table,
        "has_heading": has_heading,
        "has_list": has_list,
        "render_kind": markdown_render_kind(raw, rendered),
        "diff_stats": markdown_diff_stats_json(rendered),
    })
}

pub(crate) fn markdown_render_kind(raw: &str, rendered: &str) -> &'static str {
    if rendered.contains("[diff]") || raw.contains("kind=diff_updated") {
        "diff"
    } else if rendered.contains("[code") {
        "code"
    } else if rendered.contains("[table]") {
        "table"
    } else if raw.contains("status=failed") || raw.contains("error=") {
        "error"
    } else if raw.contains("kind=review") || raw.contains("finding") {
        "review"
    } else {
        "markdown"
    }
}

pub(crate) fn markdown_diff_stats_json(rendered: &str) -> serde_json::Value {
    let mut additions = 0usize;
    let mut deletions = 0usize;
    let mut hunks = 0usize;
    for line in rendered.lines() {
        if line.starts_with("+") {
            additions += 1;
        } else if line.starts_with("-") {
            deletions += 1;
        } else if line.trim_start().starts_with("@@") {
            hunks += 1;
        }
    }
    serde_json::json!({
        "additions": additions,
        "deletions": deletions,
        "hunks": hunks,
    })
}

pub(crate) fn markdown_unordered_item_like(trimmed: &str) -> bool {
    ["- ", "* ", "+ "]
        .into_iter()
        .any(|prefix| trimmed.starts_with(prefix))
}

pub(crate) fn markdown_ordered_item_like(trimmed: &str) -> bool {
    let Some(dot) = trimmed.find('.') else {
        return false;
    };
    let (number, rest) = trimmed.split_at(dot);
    number.chars().all(|ch| ch.is_ascii_digit()) && rest.starts_with(". ")
}

pub(crate) fn wrap_single_line(input: &str, width: usize) -> Vec<String> {
    let width = width.max(8);
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in input.split_whitespace() {
        if word.chars().count() > width {
            if !current.is_empty() {
                lines.push(current);
                current = String::new();
            }
            let mut chunk = String::new();
            for ch in word.chars() {
                if chunk.chars().count() == width {
                    lines.push(chunk);
                    chunk = String::new();
                }
                chunk.push(ch);
            }
            if !chunk.is_empty() {
                current = chunk;
            }
        } else if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current);
            current = word.to_owned();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push("none".into());
    }
    lines
}

pub(crate) fn fit(input: String, width: usize) -> String {
    let mut output = input.chars().take(width).collect::<String>();
    let padding = width.saturating_sub(output.chars().count());
    output.push_str(&" ".repeat(padding));
    output
}
